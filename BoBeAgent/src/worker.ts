import { join, isAbsolute, relative } from "node:path";
import { randomUUID } from "node:crypto";
import { Type } from "typebox";
import { VERSION, defineTool } from "@earendil-works/pi-coding-agent";
import { ModelProfiles } from "./models.js";
import { Sessions, type SessionBundle } from "./sessions.js";
import { assertActive, failureDetail, type Redactor } from "./errors.js";
import { authorityPath, privateDirectory } from "./files.js";
import {
  HARNESS_VERSION, PROTOCOL_VERSION, abortSchema, authSubmitSchema, auxiliarySchema,
  backgroundSchema, chatSchema, compactSchema, contextStatusSchema, emptySchema, initializeSchema, json, mcpToolsSchema, object,
  profileRequestSchema, probeSchema, reloadSchema, steerSchema, validate,
  type Auxiliary, type Chat, type Initialize, type Json, type JsonObject,
  type Notification, type Request, type Route,
} from "./protocol.js";
import type { RpcHandler } from "./rpc.js";
import { ProcessClient } from "./process.js";
import type { Workload, ExecutionPriority } from "./execution.js";
import { OWNER_CHANNEL } from "./persistence.js";

export interface HostConnection {
  redactor: Redactor;
  call(method: string, params: JsonObject, signal?: AbortSignal): Promise<Json>;
  notify(notification: Notification): void;
}

interface Activity {
  controller: AbortController;
  completion: Promise<Json>;
  cleanupFailures: unknown[];
}

export class WorkerHost implements RpcHandler {
  private readonly processes: ProcessClient;
  private readonly active = new Map<number, Activity>();
  private readonly unreleased = new Map<number, unknown[]>();
  private lastRequest = 0;
  private initializeAttempted = false;
  private config: Initialize | undefined;
  private models: ModelProfiles | undefined;
  private sessions: Sessions | undefined;
  private readonly channelRequests = new Map<string, number>();
  private closing = false;
  private closePromise: Promise<void> | undefined;
  private idleTimer: ReturnType<typeof setInterval> | undefined;

  constructor(
    private readonly connection: HostConnection,
    private readonly createModels: typeof ModelProfiles.create = ModelProfiles.create,
  ) {
    this.processes = new ProcessClient((method, params) => connection.call(method, params));
  }

  notification(method: string, params: JsonObject): void {
    if (method !== "process_event") throw new Error("Unknown host notification");
    this.processes.event(params);
  }

  disconnect(): void {
    this.processes.disconnect();
  }

  private notify(notification: Notification): void {
    if (!this.closing) this.connection.notify(notification);
  }

  async request(request: Request): Promise<Json> {
    if (this.closing) throw new Error("Worker is shutting down");
    const { id, method, params } = request;
    if (!Number.isSafeInteger(id) || id <= this.lastRequest) throw new Error("Duplicate or out-of-order worker request id");
    this.lastRequest = id;
    try {
      if (method === "initialize") {
        const config = validate(initializeSchema, params, "initialize parameters");
        if (this.initializeAttempted) throw new Error("Worker initialize may only run once");
        this.initializeAttempted = true;
        return await this.activity(id, signal => this.initialize(config, signal));
      }
      if (method === "shutdown") {
        validate(emptySchema, params, "shutdown parameters");
        await this.close();
        return {};
      }
      if (method === "abort") {
        const { request_id } = validate(abortSchema, params, "abort parameters");
        const activity = this.active.get(request_id);
        if (activity) {
          activity.controller.abort();
          await Promise.allSettled([activity.completion]);
        } else if (request_id >= id) {
          throw new Error("Unknown execution request id");
        }
        const unreleased = this.unreleased.get(request_id);
        this.unreleased.delete(request_id);
        if (unreleased) throw new Error(`Execution did not release its resources: ${failureDetail(unreleased)}`);
        return {};
      }
      const config = this.config;
      const models = this.models;
      const sessions = this.sessions;
      if (!config || !models || !sessions) throw new Error("Worker is not initialized");
      switch (method) {
        case "steer": {
          const control = validate(steerSchema, params, "run input parameters");
          const active = this.active.get(control.request_id);
          if (!active || active.controller.signal.aborted) throw new Error("Run is no longer accepting input");
          await sessions.steer(control.request_id, control.text, control.follow_up);
          return { queued: true };
        }
        case "background": {
          const job = validate(backgroundSchema, params, "background job parameters");
          return this.activity(id, signal => sessions.background(id, job, signal), job.budget.timeout_seconds * 1000, "goals", "background");
        }
        case "context_status": {
          const status = validate(contextStatusSchema, params, "context status parameters");
          return sessions.contextStatus(status.session_key);
        }
        case "compact": {
          const compact = validate(compactSchema, params, "compaction parameters");
          return await this.channelActivity(id, compact.session_key, signal =>
            sessions.compact(compact.keep_recent_tokens ?? undefined, signal, compact.session_key));
        }
        case "chat": {
          const chat = validate(chatSchema, params, "chat parameters");
          return await this.channelActivity(id, chat.session_key,
            signal => this.chat(id, chat, sessions, config, signal), undefined, chat.priority);
        }
        case "auxiliary": {
          const auxiliary = validate(auxiliarySchema, params, "auxiliary parameters");
          return await this.activity(id, signal => this.auxiliary(id, auxiliary, sessions, config, signal),
            undefined, auxiliary.class, "background");
        }
        case "probe": {
          const probe = validate(probeSchema, params, "probe parameters");
          return await this.activity(id, signal =>
            this.probe(id, sessions, probe.route ?? config.routes.chat, signal),
          undefined, "small", "background");
        }
        case "reload": {
          const reload = validate(reloadSchema, params, "reload parameters");
          return await this.activity(id, async signal => {
            assertActive(signal);
            await sessions.reload(reload.mcp, reload.skill_directories, signal);
            return {};
          });
        }
        case "mcp_status":
          validate(emptySchema, params, "mcp_status parameters");
          return sessions.useChannel(undefined, async bundle => bundle.mcp.status());
        case "mcp_tools": {
          const tools = validate(mcpToolsSchema, params, "mcp_tools parameters");
          return sessions.useChannel(undefined, async bundle => bundle.mcp.tools(tools.server_name));
        }
        case "auth_status": {
          const auth = validate(profileRequestSchema, params, "auth_status parameters");
          return await this.activity(id, signal => models.status(auth.profile, signal));
        }
        case "models": {
          const request = validate(profileRequestSchema, params, "models parameters");
          return await this.activity(id, signal => models.models(request.profile, signal));
        }
        case "auth_login": {
          const auth = validate(profileRequestSchema, params, "auth_login parameters");
          return await this.activity(id, signal => models.login(auth.profile, signal));
        }
        case "auth_submit": {
          const auth = validate(authSubmitSchema, params, "auth_submit parameters");
          return models.submit(auth.profile, auth.value);
        }
        case "auth_cancel": {
          const auth = validate(profileRequestSchema, params, "auth_cancel parameters");
          return await models.cancel(auth.profile);
        }
        case "auth_logout": {
          const auth = validate(profileRequestSchema, params, "auth_logout parameters");
          return await this.activity(id, signal => models.logout(auth.profile, signal));
        }
        default:
          throw new Error(`Unknown worker method: ${method}`);
      }
    } catch (error) {
      if (method === "chat") {
        this.notify({
          method: "event",
          params: { request_id: id, delta: { type: "error", message: this.connection.redactor.message(error) } },
        });
      }
      throw error;
    }
  }

  private activity(
    id: number, operation: (signal: AbortSignal) => Promise<Json>, timeoutMs?: number,
    workload: Workload = "chat", priority: ExecutionPriority = "interactive",
  ): Promise<Json> {
    if (this.active.has(id)) throw new Error("Duplicate worker request id");
    const controller = new AbortController();
    const cleanupFailures: unknown[] = [];
    const deadline = timeoutMs ?? this.models?.execution.timeout(workload);
    const timeout = deadline === undefined ? undefined : setTimeout(() =>
      controller.abort(new DOMException(`Execution exceeded ${deadline / 1_000} seconds`, "TimeoutError")), deadline);
    const completion = Promise.resolve().then(() => {
      assertActive(controller.signal);
      return this.models
        ? this.models.execution.run(workload, priority, () => operation(controller.signal))
        : operation(controller.signal);
    }).finally(() => {
      if (timeout) clearTimeout(timeout);
      this.active.delete(id);
      if (cleanupFailures.length) {
        this.unreleased.set(id, cleanupFailures);
        for (const stale of [...this.unreleased.keys()].slice(0, -64)) this.unreleased.delete(stale);
      }
    });
    this.active.set(id, { controller, completion, cleanupFailures });
    return completion;
  }

  private async channelActivity(
    id: number, channel: string | undefined, operation: (signal: AbortSignal) => Promise<Json>,
    timeoutMs?: number, priority: ExecutionPriority = "interactive",
  ): Promise<Json> {
    const key = channel ?? OWNER_CHANNEL;
    if (this.channelRequests.has(key)) throw new Error("This conversation channel is busy");
    this.channelRequests.set(key, id);
    try { return await this.activity(id, operation, timeoutMs, "chat", priority); }
    finally { this.channelRequests.delete(key); }
  }

  private async initialize(config: Initialize, signal: AbortSignal): Promise<Json> {
    if (VERSION !== HARNESS_VERSION) throw new Error("Installed pi version differs from the worker protocol");
    if (!isAbsolute(config.data_root)) throw new Error("data_root must be absolute");
    for (const path of [config.workspace, ...config.skill_directories]) {
      const child = relative(config.data_root, path);
      if (!isAbsolute(path) || child === ".." || child.startsWith("../") || isAbsolute(child)) {
        throw new Error("Worker resources must be inside the owner authority");
      }
    }
    await privateDirectory(config.data_root);
    await authorityPath(config.data_root, config.data_root);
    await privateDirectory(config.workspace);
    for (const directory of config.skill_directories) await privateDirectory(directory);
    await authorityPath(config.data_root, config.workspace);
    const agentDir = join(config.data_root, "agent");
    await privateDirectory(agentDir);
    const runtimeFiles = join(agentDir, "runtime-files");
    await privateDirectory(runtimeFiles);
    process.env.PI_CODING_AGENT_DIR = agentDir;
    process.env.PI_OFFLINE = "1";
    process.env.TMPDIR = runtimeFiles;
    process.env.PI_SKIP_VERSION_CHECK = "1";
    assertActive(signal);
    this.models = await this.createModels(
      config, agentDir,
      (profile, phase) => this.notify({ method: "auth_event", params: { profile, phase } }),
      this.connection.redactor, signal,
    );
    this.sessions = new Sessions(config, agentDir, this.models, {
      domain: (name, args, requestId, signal) => this.connection.call(
        "domain_tool", { request_id: requestId, name, arguments: args }, signal),
      event: (requestId, delta) => this.notify({ method: "event", params: { request_id: requestId, delta } }),
      activity: (requestId, name) => this.notify({ method: "tool_activity", params: { request_id: requestId, name } }),
      job: (action, args, sourceKey, requestId, signal) => this.connection.call("background_job", {
        request_id: requestId, name: action, arguments: args,
        ...(sourceKey ? { source_key: sourceKey } : {}),
      }, signal),
      schedule: (action,args,sourceKey,requestId,signal) => this.connection.call("schedule", {
        request_id:requestId,name:action,arguments:args,source_key:sourceKey,
      },signal),
      ownerInput: (action,args,sourceKey,requestId,signal) => this.connection.call("owner_input", {
        request_id:requestId,name:action,arguments:args,source_key:sourceKey,
      },signal),
      cleanupFailed: (requestId, error) => { this.active.get(requestId)?.cleanupFailures.push(error); },
    }, this.connection.redactor, this.processes);
    try {
      await this.sessions.main();
      assertActive(signal);
      this.config = config;
      this.idleTimer = setInterval(() => {
        void this.sessions?.retireIdle().catch(error => {
          process.stderr.write(`BoBe idle session cleanup failed: ${this.connection.redactor.message(error)}\n`);
        });
      }, 10_000);
      this.idleTimer.unref();
      return { protocol_version: PROTOCOL_VERSION, harness: "pi" };
    } catch (error) {
      await this.sessions.close();
      await this.models.close();
      throw error;
    }
  }

  private async chat(
    id: number, chat: Chat, sessions: Sessions, config: Initialize, signal: AbortSignal,
  ): Promise<Json> {
    if (!chat.text.trim()) throw new Error("Chat requires text");
    return sessions.useChannel(chat.session_key, async main => {
    await sessions.select(main, config.routes.chat, chat.channel, signal);
    sessions.context(main, id, chat.context, chat.channel, chat.memory_context, chat.persona);
    const events = await sessions.run(main, chat.text, signal, true);
    assertActive(signal);
    if (main.current.deferredInput) events.commitOwnerQuestion(main.current.deferredInput.prompt);
    else events.commit();
    return {};
    });
  }

  private async auxiliary(
    id: number, params: Auxiliary, sessions: Sessions, config: Initialize, signal: AbortSignal,
  ): Promise<Json> {
    let output: Json | undefined;
    const submit = defineTool({
      name: "bobe_submit_result",
      label: "Submit structured result",
      description: "Submit the requested result value for this bounded operation. Call once, when complete.",
      parameters: Type.Object({ output: Type.Unknown() }, { additionalProperties: false }),
      execute: async (_id, args, toolSignal) => {
        assertActive(toolSignal);
        if (output !== undefined) throw new Error("Structured result was already submitted");
        output = json(args.output);
        return { content: [{ type: "text", text: "Structured result accepted" }], details: {}, terminate: true };
      },
    });
    let bundle: SessionBundle | undefined;
    try {
      bundle = await sessions.finite(
        params.class === "consolidate" ? config.routes.consolidate : config.routes.small ?? config.routes.goals, id, params.context,
        "You are a bounded operation of BoBe, not another companion. Follow the job instructions. Call bobe_submit_result with the result itself as its output, or reply with only the result as JSON. Do not claim changes to owner state; the daemon applies the result.",
        [submit], signal,
      );
      const result = await sessions.run(bundle,
        `${params.job.instructions}\n\nJob kind: ${params.job.kind}\nJob input:\n${JSON.stringify(params.job.input)}`,
        signal, false, true, 8);
      if (result.failedTools > 0) throw new Error("Structured-result submission failed");
      if (output === undefined) {
        const text = result.text().trim().replace(/^```(?:json)?\s*\n([\s\S]*?)\n?```$/i, "$1");
        output = json(JSON.parse(text) as unknown);
      }
      return { job_id: params.job.job_id, output, error: null };
    } catch (error) {
      assertActive(signal);
      return { job_id: params.job.job_id, output: null, error: this.connection.redactor.message(error) };
    } finally {
      if (bundle) await sessions.release(bundle);
    }
  }

  private async probe(id: number, sessions: Sessions, route: Route, signal: AbortSignal): Promise<Json> {
    const nonce = randomUUID();
    let toolExecution = false;
    let modelExecution = false;
    let streaming = false;
    const echo = defineTool({
      name: "bobe_probe_echo",
      label: "Probe echo",
      description: "Echo a diagnostic nonce. This has no side effects.",
      parameters: Type.Object({ value: Type.Literal(nonce) }, { additionalProperties: false }),
      execute: async (_id, args, toolSignal) => {
        assertActive(toolSignal);
        toolExecution = true;
        return { content: [{ type: "text", text: args.value }], details: {} };
      },
    });
    let bundle: SessionBundle | undefined;
    try {
      bundle = await sessions.finite(
        route, id, "",
        "This is a finite model, streaming, and tool diagnostic. Call the supplied echo tool exactly once with the requested value. After its result, reply with that value. You have no companion context or personal tools.",
        [echo], signal,
      );
      const events = await sessions.run(bundle, `Call bobe_probe_echo with value ${nonce}, then repeat its result.`, signal, false, false, 4);
      modelExecution = events.last !== undefined;
      streaming = events.textDeltas > 0;
      if (!toolExecution || events.failedTools || !streaming || !events.text().includes(nonce)) {
        throw new Error("Provider did not complete the streamed echo-tool diagnostic");
      }
      return { model_execution: modelExecution, streaming, tool_execution: toolExecution, error: null };
    } catch (error) {
      assertActive(signal);
      return {
        model_execution: modelExecution, streaming, tool_execution: toolExecution,
        error: this.connection.redactor.message(error),
      };
    } finally {
      if (bundle) await sessions.release(bundle);
    }
  }

  close(): Promise<void> {
    this.closePromise ??= this.shutdown();
    return this.closePromise;
  }

  private async shutdown(): Promise<void> {
    this.closing = true;
    if (this.idleTimer) clearInterval(this.idleTimer);
    for (const activity of this.active.values()) activity.controller.abort();
    await this.models?.close();
    await Promise.allSettled([...this.active.values()].map(activity => activity.completion));
    await this.sessions?.close();
    this.processes.disconnect();
  }
}

export function initializeParams(config: Initialize): JsonObject {
  return object(json(config));
}
