import { mkdir, readFile, rm } from "node:fs/promises";
import { resolve, join } from "node:path";
import { randomUUID } from "node:crypto";
import type { TestContext } from "node:test";
import {
  createAssistantMessageEventStream, createProvider, type Api, type AssistantMessage,
  type Model, type SimpleStreamOptions, type TranscriptContext,
} from "@earendil-works/pi-ai";
import { Redactor, assertActive } from "./errors.js";
import { ModelProfiles } from "./models.js";
import { WorkerHost, initializeParams, type HostConnection } from "./worker.js";
import { initializeSchema, object, validate, toolNames, type Initialize, type Json, type Notification } from "./protocol.js";
import { ProcessFixture } from "./process-fixture.js";

const cleanups = new Map<string, (() => Promise<void>)[]>();
const contract = object(JSON.parse(await readFile(new URL("../fixtures/host-contract-v1.json", import.meta.url), "utf8")));
const initialization = validate(initializeSchema, object(contract.initialize_request).params, "shared initialization fixture");

export function deferred<T>(): {
  promise: Promise<T>;
  resolve(value: T): void;
  reject(error: Error): void;
} {
  let resolve: (value: T) => void = () => { throw new Error("Deferred is not initialized"); };
  let reject: (error: Error) => void = () => { throw new Error("Deferred is not initialized"); };
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

export async function fixture(t: TestContext): Promise<{ root: string; workspace: string; skills: string }> {
  const relative = join(".test-state", randomUUID());
  await mkdir(relative, { recursive: true, mode: 0o700 });
  const root = resolve(relative);
  const workspace = join(root, "workspace");
  const skills = join(root, "skills/learned");
  await mkdir(workspace, { mode: 0o700 });
  await mkdir(skills, { recursive: true, mode: 0o700 });
  const cleanup: (() => Promise<void>)[] = [];
  cleanups.set(root, cleanup);
  t.after(async () => {
    try {
      for (const dispose of cleanup.reverse()) await dispose();
    } finally {
      cleanups.delete(root);
      await rm(relative, { recursive: true, force: true });
    }
  });
  return { root, workspace, skills };
}

export function configuration(paths: { root: string; workspace: string; skills: string }): Initialize {
  const route = { profile: "test", model: "fixture-model", reasoning_effort: "medium" as const };
  return {
    execution: structuredClone(initialization.execution),
    workload_budgets: structuredClone(initialization.workload_budgets),
    protocol_version: 1, data_root: paths.root, workspace: paths.workspace,
    skill_directories: [paths.skills],
    profiles: { test: { kind: "open_ai", api_key: "fixture-api-key", auth_method: "api_key" } },
    routes: { chat: { ...route }, goals: { ...route }, consolidate: { ...route } },
    tools: [], capabilities: { memory: true, goals: true }, mcp: { servers: {} },
    enabled_tools: [...toolNames],
  };
}

const fixtureModel: Model<Api> = {
  id: "fixture-model", name: "Controlled SDK test model", api: "openai-completions",
  provider: "bobe-profile:test", baseUrl: "http://127.0.0.1:1/v1",
  input: ["text", "image"], reasoning: true, contextWindow: 100_000, maxTokens: 4_096,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
};

interface ScriptResponse {
  text?: string;
  thinking?: string;
  tools?: { id: string; name: string; arguments: { [key: string]: Json } }[];
  error?: string;
  stopReason?: Exclude<AssistantMessage["stopReason"], "pending">;
  inputTokens?: number;
  outputTokens?: number;
}

type ModelScript = (
  context: TranscriptContext, options: SimpleStreamOptions | undefined, model: Model<Api>,
) => ScriptResponse | Promise<ScriptResponse>;

export function controlledProvider(
  script: ModelScript, providerId = fixtureModel.provider,
  limits: { contextWindow?: number; maxTokens?: number } = {},
) {
  const model = { ...fixtureModel, ...limits, provider: providerId };
  const stream = (selected: Model<Api>, context: TranscriptContext, options?: SimpleStreamOptions) => {
    const events = createAssistantMessageEventStream();
    void (async () => {
      const message: AssistantMessage = {
        role: "assistant", api: selected.api, provider: selected.provider, model: selected.id,
        content: [], timestamp: Date.now(), stopReason: "stop",
        usage: {
          input: 1, output: 2, cacheRead: 0, cacheWrite: 0, totalTokens: 3,
          cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
        },
      };
      try {
        const result = await script(context, options, selected);
        assertActive(options?.signal);
        message.usage.input = result.inputTokens ?? message.usage.input;
        message.usage.output = result.outputTokens ?? message.usage.output;
        message.usage.totalTokens = message.usage.input + message.usage.output;
        events.push({ type: "start", partial: message });
        if (result.thinking) {
          const index = message.content.length;
          message.content.push({ type: "thinking", thinking: result.thinking });
          events.push({ type: "thinking_start", contentIndex: index, partial: message });
          events.push({ type: "thinking_delta", contentIndex: index, delta: result.thinking, partial: message });
          events.push({ type: "thinking_end", contentIndex: index, content: result.thinking, partial: message });
        }
        if (result.text !== undefined) {
          const index = message.content.length;
          const content = { type: "text" as const, text: "" };
          message.content.push(content);
          events.push({ type: "text_start", contentIndex: index, partial: message });
          for (const chunk of result.text.match(/.{1,5}/gs) ?? []) {
            content.text += chunk;
            events.push({ type: "text_delta", contentIndex: index, delta: chunk, partial: message });
          }
          events.push({ type: "text_end", contentIndex: index, content: result.text, partial: message });
        }
        for (const tool of result.tools ?? []) {
          const index = message.content.length;
          const toolCall = { type: "toolCall" as const, ...tool };
          message.content.push(toolCall);
          events.push({ type: "toolcall_start", contentIndex: index, partial: message });
          events.push({ type: "toolcall_end", contentIndex: index, toolCall, partial: message });
        }
        if (result.error) {
          message.stopReason = "error";
          message.errorMessage = result.error;
          events.push({ type: "error", reason: "error", error: message });
        } else {
          const reason = result.stopReason ?? (result.tools?.length ? "toolUse" : "stop");
          if (reason === "error" || reason === "aborted") {
            message.stopReason = reason;
            events.push({ type: "error", reason, error: message });
          } else {
            message.stopReason = reason;
            events.push({ type: "done", reason, message });
          }
        }
        events.end(message);
      } catch (error) {
        message.stopReason = options?.signal?.aborted ? "aborted" : "error";
        message.errorMessage = error instanceof Error ? error.message : "Controlled provider failed";
        events.push({ type: "error", reason: message.stopReason, error: message });
        events.end(message);
      }
    })();
    return events;
  };
  return createProvider({
    id: providerId, name: "Controlled SDK adapter", baseUrl: model.baseUrl,
    models: [model],
    auth: {
      apiKey: {
        name: "Test credential",
        resolve: async ({ credential }) => credential?.key ? { auth: { apiKey: credential.key } } : undefined,
      },
    },
    api: { stream, streamSimple: stream },
  });
}

export async function host(
  t: TestContext, config: Initialize, script: ModelScript,
  call: HostConnection["call"] = async () => ({ success: true, value: {} }),
  modelLimits: { contextWindow?: number; maxTokens?: number } = {},
  processCapacity?: number,
): Promise<{ worker: WorkerHost; notifications: Notification[]; profiles: ModelProfiles; processes:ProcessFixture }> {
  const notifications: Notification[] = [];
  let profiles: ModelProfiles | undefined;
  const processes = new ProcessFixture(params => worker.notification("process_event", params), processCapacity);
  const worker = new WorkerHost({
    call: (method, params, signal) => method === "process" ? processes.invoke(params) : call(method, params, signal),
    notify: value => notifications.push(value),
    redactor: new Redactor(),
  }, async (...args) => {
    profiles = await ModelProfiles.create(...args);
    for (const profile of Object.keys(config.profiles)) {
      profiles.registerNativeProvider(controlledProvider(script, `bobe-profile:${profile}`, modelLimits));
    }
    return profiles;
  });
  const cleanup = cleanups.get(config.data_root);
  const close = async (): Promise<void> => {
    try { await worker.close(); } finally { await processes.close(); }
  };
  if (cleanup) cleanup.push(close);
  else t.after(close);
  await worker.request({ id: 1, method: "initialize", params: initializeParams(config) });
  if (!profiles) throw new Error("Controlled profiles were not initialized");
  return { worker, notifications, profiles,processes };
}

export function userText(context: TranscriptContext): string {
  const message = context.messages.findLast(message => message.role === "user");
  if (!message || message.role !== "user") return "";
  const text = typeof message.content === "string"
    ? message.content : message.content.filter(part => part.type === "text").map(part => part.text).join("");
  return text.replace(/^\[bobe\.turn_context\]\n[\s\S]*?\n\[\/bobe\.turn_context\]\n\n/, "");
}
