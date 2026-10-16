import { readFile } from "node:fs/promises";
import {
  compact as compactSession, createAgentSession, createBashToolDefinition, createReadToolDefinition,
  createEditToolDefinition, createWriteToolDefinition,
  createLsToolDefinition, defineTool, SessionManager, SettingsManager,
  type AgentSession, type ExtensionFactory, type ToolDefinition,
} from "@earendil-works/pi-coding-agent";
import type { Api, Model } from "@earendil-works/pi-ai";
import { Type } from "typebox";
import { AuthorityResources } from "./resources.js";
import { McpResources } from "./mcp.js";
import { ModelProfiles } from "./models.js";
import { TurnEvents } from "./events.js";
import { boundedOutput, subagentTool, type DelegateTask, type DelegateResult } from "./delegation.js";
import { Cancelled, Serial, WorkerFault, assertActive, failureDetail, settleEach, type Redactor } from "./errors.js";
import { createChildSession, flushSession, openBackgroundSession, openChannelSession, OWNER_CHANNEL } from "./persistence.js";
import { backgroundTools } from "./background.js";
import { scheduleTool } from "./scheduling.js";
import { ownerInputTool, type DeferredOwnerInput } from "./owner-input.js";
import { ownedFind, ownedGrep } from "./search.js";
import { ProcessClient } from "./process.js";
import {
  domainResultSchema, isRecord, json, object, validate, type Delta, type Initialize,
  type BackgroundRequest, type Json, type JsonObject, type McpConfig, type Persona, type Route, type Thinking,
} from "./protocol.js";

let packagedBehavior: Promise<string> | undefined;

function companionBehavior(): Promise<string> {
  packagedBehavior ??= readFile(new URL("../assets/chat.md", import.meta.url), "utf8").then(text => {
    if (!text.trim()) throw new Error("The packaged companion behavior is empty");
    return text.trim();
  });
  return packagedBehavior;
}

function capabilityFacts(capabilities: Initialize["capabilities"]): string {
  return Object.entries(capabilities).map(([name, enabled]) => `${name}: ${enabled ? "on" : "off"}`).join("\n");
}

function inertMarkers(text: string): string {
  return text.replace(/\[(?=\/?bobe\.)/gi, "\\[");
}

function inertDocument(document: string): string {
  return inertMarkers(document).replace(/<(?=\/?(?:bobe_|soul\b|profile\b))/gi, "&lt;");
}

function personaSection(persona: Persona): string {
  const entries = (tag: string, documents: string[]): string[] =>
    documents.map(document => document.trim()).filter(Boolean).map(document => `<${tag}>\n${inertDocument(document)}\n</${tag}>`);
  return [...entries("soul", persona.souls), ...entries("profile", persona.profiles)].join("\n");
}

const noPersona: Persona = { souls: [], profiles: [] };

function rejections(outcomes: readonly PromiseSettledResult<unknown>[]): unknown[] {
  return outcomes.flatMap(outcome => outcome.status === "rejected" ? [outcome.reason] : []);
}

interface SessionCallbacks {
  domain(name: string, args: JsonObject, requestId: number, signal?: AbortSignal): Promise<Json>;
  event(requestId: number, delta: Delta): void;
  activity(requestId: number, name: string): void;
  job(action: string, args: JsonObject, sourceKey: string | undefined, requestId: number, signal?: AbortSignal): Promise<Json>;
  schedule(action: string, args: JsonObject, sourceKey: string, requestId: number, signal?: AbortSignal): Promise<Json>;
  ownerInput(action: string, args: JsonObject, sourceKey: string, requestId: number, signal?: AbortSignal): Promise<Json>;
  cleanupFailed(requestId: number, error: unknown): void;
}

interface TurnContext {
  requestId: number;
  context: string;
  memoryContext: string;
  persona: Persona;
  channel: "voice" | "text";
  depth: number;
  childrenCreated: { count: number };
  visibleOutput: boolean;
  deferredInput?: DeferredOwnerInput;
}

export class SessionBundle {
  current: TurnContext = {
    requestId: 0, context: "", memoryContext: "", persona: noPersona, channel: "text", depth: 0,
    childrenCreated: { count: 0 }, visibleOutput: false,
  };
  readonly errors: string[] = [];
  private closePromise: Promise<void> | undefined;
  private readonly resourceChanges = new Serial();

  constructor(
    readonly session: AgentSession,
    readonly resources: AuthorityResources,
    readonly mcp: McpResources,
    readonly fullCapabilities: boolean,
  ) {}

  close(): Promise<void> {
    this.closePromise ??= this.resourceChanges.run(() => this.dispose());
    return this.closePromise;
  }

  async refreshSkills(directories: string[]): Promise<void> {
    await this.resourceChanges.run(async () => {
      if (this.closePromise) return;
      await this.resources.refreshSkills(directories);
      this.session.setActiveToolsByName(this.session.getActiveToolNames());
    });
  }

  private async dispose(): Promise<void> {
    const previousErrors = this.errors.length;
    const failures = await settleEach([
      () => this.session.abort(),
      () => this.session.extensionRunner.emit({ type: "session_shutdown", reason: "quit" }),
      () => this.mcp.close(),
      () => this.session.settingsManager.flush(),
      () => flushSession(this.session.sessionManager),
    ]);
    this.session.dispose();
    const cleanupErrors = this.errors.slice(previousErrors);
    if (cleanupErrors.length) failures.push(new Error(`Session extension cleanup failed: ${cleanupErrors.join("; ")}`));
    if (failures.length) throw new AggregateError(failures, "Session cleanup failed");
  }
}

const builtins = ["read", "bash", "edit", "write", "grep", "find", "ls"] as const;

export class Sessions {
  readonly bundles = new Set<SessionBundle>();
  private readonly skillRefresh = new Serial();
  private readonly channels = new Map<string, Promise<SessionBundle>>();
  private readonly channelAccess = new Map<string, number>();
  private readonly channelLeases = new Map<string, number>();
  private readonly retiringChannels = new Map<string, Promise<void>>();
  private readonly channelChanges = new Serial();
  private maintenance = 0;
  private readonly maintenanceWaiters: (() => void)[] = [];
  private skillVersion = 0;
  private mcpConfig: McpConfig;
  private skillDirectories: string[];
  private readonly activeRuns = new Set<Promise<TurnEvents>>();
  private closed = false;

  constructor(
    private readonly config: Initialize,
    readonly agentDir: string,
    readonly models: ModelProfiles,
    private readonly callbacks: SessionCallbacks,
    private readonly redactor: Redactor,
    private readonly processes: ProcessClient,
  ) {
    this.mcpConfig = config.mcp;
    this.skillDirectories = config.skill_directories;
  }

  async main(channel = OWNER_CHANNEL): Promise<SessionBundle> {
    if (this.closed) throw new Error("Sessions are closed");
    await this.retiringChannels.get(channel);
    const existing = this.channels.get(channel);
    if (existing) {
      this.channelAccess.set(channel, Date.now());
      return existing;
    }
    return this.channelChanges.run(async () => {
      const raced = this.channels.get(channel);
      if (raced) return raced;
      while (this.channels.size >= this.models.execution.limits.live_channels) {
        const idle = [...this.channels.keys()]
          .filter(key => !this.channelLeases.get(key))
          .sort((a, b) => (this.channelAccess.get(a) ?? 0) - (this.channelAccess.get(b) ?? 0))[0];
        if (!idle) throw new Error("All live conversation slots are active; wait for a turn to settle");
        await this.retireChannel(idle);
      }
      const created = (async () => {
        const manager = await openChannelSession(this.agentDir, this.config.workspace, channel);
        return this.create(manager, true, undefined, [], undefined, true);
      })();
      this.channels.set(channel, created);
      this.channelAccess.set(channel, Date.now());
      try { return await created; }
      catch (error) {
        this.channels.delete(channel);
        this.channelAccess.delete(channel);
        throw error;
      }
    });
  }

  async useChannel<T>(channel: string | undefined, operation: (bundle: SessionBundle) => Promise<T>): Promise<T> {
    while (this.maintenance) await new Promise<void>(resolve => this.maintenanceWaiters.push(resolve));
    if (this.closed) throw new Error("Sessions are closed");
    const key = channel ?? OWNER_CHANNEL;
    this.channelLeases.set(key, (this.channelLeases.get(key) ?? 0) + 1);
    try { return await operation(await this.main(key)); }
    finally {
      const remaining = (this.channelLeases.get(key) ?? 1) - 1;
      if (remaining) this.channelLeases.set(key, remaining);
      else this.channelLeases.delete(key);
      this.channelAccess.set(key, Date.now());
    }
  }

  private async retireChannel(channel: string): Promise<void> {
    const existing = this.channels.get(channel);
    if (!existing) return;
    const closing = (async () => {
      try { await this.release(await existing); }
      finally {
        this.channels.delete(channel);
        this.channelAccess.delete(channel);
        this.retiringChannels.delete(channel);
      }
    })();
    this.retiringChannels.set(channel, closing);
    await closing;
  }

  async retireIdle(): Promise<void> {
    if (this.closed || this.maintenance) return;
    await this.channelChanges.run(async () => {
      if (this.closed || this.maintenance) return;
      const cutoff = Date.now() - this.models.execution.limits.idle_channel_seconds * 1000;
      for (const channel of this.channels.keys()) {
        if (!this.channelLeases.get(channel) && (this.channelAccess.get(channel) ?? 0) <= cutoff) {
          await this.retireChannel(channel);
        }
      }
    });
  }

  private async create(
    manager: SessionManager,
    full: boolean,
    model?: Model<Api>,
    extraTools: ToolDefinition[] = [],
    finitePrompt?: string,
    isCanonical = false,
    mcpConfig: McpConfig = full ? this.mcpConfig : { servers: {} },
  ): Promise<SessionBundle> {
    assertActive();
    if (this.closed) throw new Error("Sessions are closed");
    const skillVersion = this.skillVersion;
    const mcp = new McpResources(mcpConfig, this.agentDir, this.redactor, this.processes,
      tool => this.models.execution.tool(tool));
    const settings = SettingsManager.inMemory({
      defaultTools: full ? builtins.filter(name => this.config.enabled_tools.includes(name)) : [],
      retry: { enabled: false, provider: { maxRetries: 0, timeoutMs: 120_000 } },
      compaction: { enabled: full },
      cacheWarming: "off",
      images: { autoResize: false },
      enableInstallTelemetry: false,
      enableAnalytics: false,
      enableSkillCommands: false,
      defaultProjectTrust: "never",
      packages: [],
      extensions: [],
      skills: [],
      prompts: [],
      themes: [],
    }, { projectTrusted: false });
    let current: SessionBundle | undefined;
    const binding = (): SessionBundle => {
      if (!current) throw new Error("Session tools are not bound");
      return current;
    };
    const contextExtension: ExtensionFactory = pi => {
      pi.on("tool_call", event => {
        const bundle=binding();
        if (bundle.current.deferredInput) return { block:true,reason:"This run is waiting for an important owner answer. Do not perform its deferred action." };
        const assistant=bundle.session.messages.findLast(message=>message.role==="assistant");
        if (assistant?.role==="assistant") {
          const question=assistant.content.find(block=>block.type==="toolCall" && block.name==="bobe_owner_input"
            && isRecord(block.arguments) && (block.arguments.operation==="ask" || block.arguments.operation==="present"));
          if (question?.type==="toolCall" && question.id!==event.toolCallId) {
            return {block:true,reason:"Settle the important owner question before starting other tools in this round."};
          }
        }
      });
      pi.on("before_agent_start", event => {
        const bundle = binding();
        const sections = event.systemPromptOptions.sections;
        if (full) {
          sections.bobe_capabilities = capabilityFacts(this.config.capabilities);
          sections.bobe_persona = personaSection(bundle.current.persona);
        }
        sections.bobe_memory = inertDocument(bundle.current.memoryContext);
        event.systemPromptOptions.skills = bundle.resources.getSkills().skills;
      });
      pi.on("ui_prompt_start", (_event, ctx) => {
        binding().errors.push("Interactive tool dialogs are not supported by the headless worker");
        ctx.abort();
      });
      pi.on("session_before_compact", async event => {
        if (event.reason === "overflow" && binding().current.visibleOutput) return { cancel: true };
        if (this.config.routes.small) {
          const model = await this.models.select(this.config.routes.small, event.signal);
          return { compaction: await compactSession(
            event.preparation, model, undefined, undefined, event.customInstructions,
            event.signal, this.config.routes.small.reasoning_effort ?? "low",
            (selected, context, options) => this.models.runtime.streamSimple(selected, context, options),
          ) };
        }
        return undefined;
      });
    };
    const resources = new AuthorityResources(
      this.config.data_root, this.config.workspace, this.agentDir,
      full ? this.skillDirectories : [], settings,
      finitePrompt ?? await companionBehavior(),
      [contextExtension, mcp.extension()],
    );
    await resources.reload();
    const customTools = full
      ? [...[
        defineTool(createReadToolDefinition(this.config.workspace)),
        defineTool(createEditToolDefinition(this.config.workspace)),
        defineTool(createWriteToolDefinition(this.config.workspace)),
        defineTool(ownedGrep(this.config.workspace,this.processes,() => binding().current.requestId,this.agentDir)),
        defineTool(ownedFind(this.config.workspace,this.processes,() => binding().current.requestId,this.agentDir)),
        defineTool(createLsToolDefinition(this.config.workspace)),
      ].filter(tool => this.config.enabled_tools.some(name => name === tool.name)),
      ...(this.config.enabled_tools.includes("bash") ? [defineTool(createBashToolDefinition(this.config.workspace, {
        operations: this.processes.shell(() => binding().current.requestId),
      }))] : []), ...this.domainTools(binding), ...(this.config.enabled_tools.includes("subagent") ? [subagentTool({
        run: (task, signal) => this.delegate(binding(), task, signal),
      })] : []), ...(this.config.enabled_tools.includes("background_jobs") ? [...backgroundTools({
        invocationScope: () => this.invocationScope(binding()),
        call: (action, args, sourceKey, signal) =>
          this.callbacks.job(action, args, sourceKey, binding().current.requestId, signal),
      }, isCanonical), scheduleTool({
        invocationScope: () => this.invocationScope(binding()),
        call: (action,args,sourceKey,signal) => {
          if (!sourceKey) throw new Error("Schedule requires a source identity");
          return this.callbacks.schedule(action,args,sourceKey,binding().current.requestId,signal);
        },
      }), ownerInputTool({
        invocationScope: () => this.invocationScope(binding()),
        call: (action,args,sourceKey,signal) => {
          if (!sourceKey) throw new Error("Owner input requires a source identity");
          if (binding().current.depth>0) throw new Error("Return the missing decision to the parent; scoped subagents cannot ask the owner directly");
          return this.callbacks.ownerInput(action,args,sourceKey,binding().current.requestId,signal);
        },
      }, input => { binding().current.deferredInput=input; })] : []), ...extraTools]
      : extraTools;
    try {
      const { session } = await createAgentSession({
        cwd: this.config.workspace,
        agentDir: this.agentDir,
        modelRuntime: this.models.runtime,
        ...(model ? { model } : {}),
        thinkingLevel: "off",
        sessionManager: manager,
        settingsManager: settings,
        resourceLoader: resources,
        customTools: customTools.map(tool => this.models.execution.tool(tool)),
        excludeTools: [
          ...builtins.filter(name => !full || !this.config.enabled_tools.includes(name)),
          ...(!full || !this.config.enabled_tools.includes("subagent") ? ["subagent"] : []),
          ...(!isCanonical ? ["background_task"] : []),
          ...(!this.config.enabled_tools.includes("background_jobs")
            ? ["background_task", "background_tasks", "background_task_status", "background_task_cancel", "bobe_schedule", "bobe_owner_input"] : []),
        ],
      });
      current = new SessionBundle(session, resources, mcp, full);
      const ui = session.extensionRunner.createContext().ui;
      const noDialog = (): never => { throw new Error("Interactive dialogs are not available in the worker"); };
      await session.bindExtensions({
        mode: "rpc",
        uiContext: {
          ...ui,
          input: noDialog,
          select: noDialog,
          confirm: noDialog,
          custom: noDialog,
          editor: noDialog,
          notify: (message, type) => {
            if (type === "error" || type === "warning") {
              binding().errors.push(this.redactor.message(new Error(message)));
            }
          },
        },
        onError: error => binding().errors.push(this.redactor.message(new Error(error.error))),
      });
      if (full && skillVersion !== this.skillVersion) await current.refreshSkills(this.skillDirectories);
      this.bundles.add(current);
      return current;
    } catch (error) {
      await mcp.close();
      if (current) {
        current.session.dispose();
        this.bundles.delete(current);
      }
      throw error;
    }
  }

  private domainTools(binding: () => SessionBundle): ToolDefinition[] {
    const names = new Set<string>();
    return this.config.tools.map(tool => {
      if (names.has(tool.name)) throw new Error("Duplicate owner domain tool");
      names.add(tool.name);
      if (tool.parameters.type !== "object") throw new Error("Domain tool parameters require an object schema");
      return defineTool({
        name: tool.name,
        label: tool.name,
        description: tool.description,
        parameters: Type.Unsafe<JsonObject>(tool.parameters),
        execute: async (_id, args, signal) => {
          const bundle = binding();
          assertActive(signal);
          const response = validate(domainResultSchema,
            await this.callbacks.domain(tool.name, object(args), bundle.current.requestId, signal),
            "domain tool result");
          assertActive(signal);
          if (!response.success) throw new Error(response.value.error);

          const value = json(response.value);
          return { content: [{ type: "text", text: JSON.stringify(value) }], details: value, structuredContent: value };
        },
      });
    });
  }

  private invocationScope(bundle: SessionBundle): string {
    const manager = bundle.session.sessionManager;
    const origin = manager.getBranch().findLast(entry => entry.type === "message" && entry.message.role === "assistant");
    if (!origin) throw new Error("Tool execution has no assistant message identity");
    return `${manager.getSessionId()}:${origin.id}`;
  }

  async refreshSkills(directories = this.skillDirectories): Promise<void> {
    this.maintenance++;
    try {
      await this.skillRefresh.run(async () => {
        this.skillDirectories = [...directories];
        this.skillVersion++;
        for (const bundle of this.bundles) {
          if (bundle.fullCapabilities) await bundle.refreshSkills(directories);
        }
      });
    } finally { this.finishMaintenance(); }
  }

  private finishMaintenance(): void {
    this.maintenance--;
    if (!this.maintenance) for (const resolve of this.maintenanceWaiters.splice(0)) resolve();
  }

  async reload(mcp?: McpConfig, directories?: string[], signal?: AbortSignal): Promise<void> {
    if (mcp && (this.activeRuns.size || this.channelLeases.size)) throw new Error("MCP refresh requires idle execution");
    this.maintenance++;
    try {
      if (directories) await this.refreshSkills(directories);
      else await this.refreshSkills();
      if (mcp) await this.channelChanges.run(() => this.replaceMcp(mcp, signal));
    } finally { this.finishMaintenance(); }
  }

  private async replaceMcp(next: McpConfig, signal?: AbortSignal): Promise<void> {
    const staged = await this.create(
      SessionManager.inMemory(this.config.workspace), false, undefined, [], "Owner MCP server verification.", false, next,
    );
    try {
      await this.connectMcp(staged, signal);
    } finally {
      await this.release(staged).catch((error: unknown) => {
        throw new WorkerFault(`MCP verification servers could not be stopped: ${failureDetail([error])}`, { cause: error });
      });
    }
    const previous = this.mcpConfig;
    const bundles = await Promise.all(this.channels.values());
    const failures = rejections(await Promise.allSettled(bundles.map(bundle => this.applyMcp(bundle, next, signal))));
    if (failures.length) {
      const restoreFailures = rejections(await Promise.allSettled(bundles.map(bundle => this.applyMcp(bundle, previous))));
      if (restoreFailures.length) {
        throw new WorkerFault(
          `MCP reload failed and the previous servers could not be restored: ${failureDetail([...failures, ...restoreFailures])}`,
        );
      }
      throw new Error(`MCP reload was rolled back: ${failureDetail(failures)}`);
    }
    this.mcpConfig = next;
  }

  private async applyMcp(bundle: SessionBundle, config: McpConfig, signal?: AbortSignal): Promise<void> {
    await bundle.mcp.replace(config);
    bundle.errors.length = 0;
    await bundle.session.reload();
    await this.connectMcp(bundle, signal);
    await flushSession(bundle.session.sessionManager);
  }

  private async connectMcp(bundle: SessionBundle, signal?: AbortSignal): Promise<void> {
    await bundle.mcp.ready(signal);
    if (bundle.errors.length) throw new Error(`MCP reload failed: ${bundle.errors.join("; ")}`);
  }

  async select(bundle: SessionBundle, route: Route, channel: "voice" | "text", signal?: AbortSignal): Promise<void> {
    const model = await this.models.select(route, signal);
    assertActive(signal);
    if (bundle.session.model?.provider !== model.provider || bundle.session.model.id !== model.id
      || bundle.session.model.contextWindow !== model.contextWindow || bundle.session.model.maxTokens !== model.maxTokens) {
      await bundle.session.setModel(model, { persist: false });
    }
    const thinking: Thinking = channel === "voice" ? "low" : route.reasoning_effort ?? "medium";
    bundle.session.setThinkingLevel(thinking, { persist: false });
  }

  context(
    bundle: SessionBundle, requestId: number, context: string, channel: "voice" | "text", memoryContext = "",
    persona = noPersona,
  ): void {
    bundle.current = {
      requestId, context, memoryContext, persona, channel, depth: 0, childrenCreated: { count: 0 }, visibleOutput: false,
    };
  }

  async steer(requestId: number, text: string, followUp: boolean): Promise<void> {
    if (!text.trim()) throw new Error("Run input must not be empty");
    const bundle = [...this.bundles].find(candidate =>
      candidate.fullCapabilities && candidate.current.depth === 0 && candidate.current.requestId === requestId);
    if (!bundle?.session.isStreaming || bundle.current.deferredInput) throw new Error("Run is not accepting input at this time");
    if (bundle.session.pendingMessageCount >= 16) throw new Error("Run input queue is full");
    if (followUp) await bundle.session.followUp(text, undefined, { source: "rpc" });
    else await bundle.session.steer(text, undefined, { source: "rpc" });
    if (bundle.session.isIdle && bundle.session.pendingMessageCount) {
      bundle.session.clearQueue();
      throw new Error("Run ended before the queued input could be applied");
    }
  }

  run(
    bundle: SessionBundle, text: string, signal: AbortSignal,
    stream: boolean, allowToolTermination = false, turnLimit = 64,
  ): Promise<TurnEvents> {
    const running = this.execute(bundle, text, signal, stream, allowToolTermination, turnLimit);
    this.activeRuns.add(running);
    void running.then(() => this.activeRuns.delete(running), () => this.activeRuns.delete(running));
    return running;
  }

  private async execute(
    bundle: SessionBundle, text: string, signal: AbortSignal,
    stream: boolean, allowToolTermination: boolean, turnLimit: number,
  ): Promise<TurnEvents> {
    assertActive(signal);
    const events = new TurnEvents(
      stream ? delta => {
        if (delta.type === "message_delta") bundle.current.visibleOutput = true;
        this.callbacks.event(bundle.current.requestId, delta);
      } : undefined,
      stream ? name => this.callbacks.activity(bundle.current.requestId, name) : undefined,
    );
    const cancellations: Promise<PromiseSettledResult<void>[]>[] = [];
    const cancelResources = (): void => {
      cancellations.push(Promise.allSettled([bundle.session.abort(), bundle.mcp.cancelActive()]));
    };
    let cancelled = false;
    const cancel = (): void => {
      if (cancelled) return;
      cancelled = true;
      cancelResources();
    };
    let budgetFailure = false;
    const unsubscribe = bundle.session.subscribe(event => {
      events.event(event);
      if (event.type === "agent_start" && (signal.aborted || budgetFailure)) cancelResources();
      if (events.turns >= turnLimit && event.type === "turn_end" &&
        event.message.role === "assistant" && event.message.stopReason === "toolUse" && !allowToolTermination && !bundle.current.deferredInput) {
        budgetFailure = true;
        cancel();
      }
    });
    signal.addEventListener("abort", cancel, { once: true });
    try {
      if (bundle.mcp.needsRestart()) {
        await bundle.mcp.prepareRestart();
        bundle.errors.length = 0;
        await bundle.session.reload();
      }
      const selectedModel = bundle.session.model;
      if (selectedModel) {
        bundle.session.settingsManager.applyOverrides({
          compaction: {
            enabled: bundle.fullCapabilities,
            reserveTokens: Math.min(16_384, selectedModel.maxTokens, Math.max(128, Math.floor(selectedModel.contextWindow / 4))),
            keepRecentTokens: Math.min(20_000, Math.max(128, Math.floor(selectedModel.contextWindow / 3))),
          },
        });
      }
      await bundle.mcp.ready(signal);
      assertActive(signal);
      if (bundle.errors.length) throw new Error(`Session extension failed: ${bundle.errors.join("; ")}`);
      const hints = [bundle.current.context];
      if (bundle.current.channel === "voice") hints.push("This is a live spoken turn.");
      const hint = hints.filter(Boolean).join("\n\n");
      const prompt = hint ? `[bobe.turn_context]\n${hint}\n[/bobe.turn_context]\n\n${text}` : text;
      await bundle.session.prompt(prompt, { source: "rpc", expandPromptTemplates: false });
      await bundle.session.waitForIdle();
      await Promise.all(cancellations);
      assertActive(signal);
      if (budgetFailure) throw new Error("pi execution exceeded the bounded turn limit");
      if (bundle.errors.length) throw new Error(`Session extension failed: ${bundle.errors.join("; ")}`);
      events.assertSuccess(allowToolTermination || bundle.current.deferredInput !== undefined);
      await flushSession(bundle.session.sessionManager);
      return events;
    } finally {
      signal.removeEventListener("abort", cancel);
      const failures = await settleEach([
        async () => {
          const failed = rejections((await Promise.all(cancellations)).flat());
          if (failed.length) throw new AggregateError(failed, "Turn cancellation failed");
        },
        async () => { if (!bundle.session.isIdle) await bundle.session.abort(); },
        () => bundle.session.clearQueue(),
        () => events.endFailedTools(),
        unsubscribe,
        () => flushSession(bundle.session.sessionManager),
      ]);
      if (failures.length) {
        const failure = new AggregateError(failures, `Run cleanup failed: ${failureDetail(failures)}`);
        this.callbacks.cleanupFailed(bundle.current.requestId, failure);
        throw failure;
      }
    }
  }

  async finite(
    route: Route, requestId: number, context: string, systemPrompt: string,
    customTools: ToolDefinition[], signal: AbortSignal,
  ): Promise<SessionBundle> {
    const model = await this.models.select(route, signal);
    assertActive(signal);
    const bundle = await this.create(SessionManager.inMemory(this.config.workspace), false, model, customTools, systemPrompt);
    try {
      this.context(bundle, requestId, context, "text");
      await this.select(bundle, route, "text", signal);
      return bundle;
    } catch (error) {
      await this.release(bundle);
      throw error;
    }
  }

  async contextStatus(channel = OWNER_CHANNEL): Promise<JsonObject> {
    return this.useChannel(channel, async bundle => this.contextSnapshot(bundle));
  }

  private contextSnapshot(bundle: SessionBundle): JsonObject {
    const stats = bundle.session.getSessionStats();
    const context = bundle.session.getContextUsage();
    const branch = bundle.session.sessionManager.getBranch();
    const last = branch.findLast(entry => entry.type === "message" && entry.message.role === "assistant");
    const usage = last?.type === "message" && last.message.role === "assistant" ? last.message.usage : undefined;
    return {
      execution: this.models.execution.snapshot(),
      live_channels: this.channels.size,
      session_id: stats.sessionId,
      profile: this.config.routes.chat.profile,
      model: this.config.routes.chat.model,
      user_messages: stats.userMessages,
      assistant_messages: stats.assistantMessages,
      tool_calls: stats.toolCalls,
      tool_results: stats.toolResults,
      tokens: {
        input: stats.tokens.input, output: stats.tokens.output,
        cache_read: stats.tokens.cacheRead, cache_write: stats.tokens.cacheWrite, total: stats.tokens.total,
      },
      last_response_tokens: usage ? {
        input: usage.input, output: usage.output, cache_read: usage.cacheRead,
        cache_write: usage.cacheWrite, total: usage.totalTokens,
      } : null,
      context_tokens: context?.tokens ?? null,
      context_window: context?.contextWindow ?? null,
      context_percent: context?.percent ?? null,
      compactions: branch.filter(entry => entry.type === "compaction").length,
      active_children: [...this.bundles].filter(candidate =>
        candidate.current.depth > 0 && candidate.current.requestId === bundle.current.requestId && !candidate.session.isIdle).length,
    };
  }

  async background(requestId: number, params: BackgroundRequest, signal: AbortSignal): Promise<JsonObject> {
    if (!this.config.enabled_tools.includes("background_jobs")) throw new Error("Background work is disabled");
    const model = await this.models.select(params.route, signal);
    assertActive(signal);
    const manager = await openBackgroundSession(this.agentDir, this.config.workspace, params.job_id);
    const bundle = await this.create(manager, true, model);
    try {
      this.context(bundle, requestId,
        `Background job ${params.job_id}: ${params.title}.\nThis is bounded delegated work. Return the outcome to the canonical companion; do not address the owner as another companion.\n${params.context}`,
        "text", params.memory_context, params.persona);
      await this.select(bundle, params.route, "text", signal);
      const prompt = params.attempt > 1
        ? `This task is being reconsidered after interruption or failure. Earlier external effects may have happened. Inspect the retained transcript and current files/tool state before repeating effects. Choose whether to continue, use another method or wait within the task's existing authorization. A retry is not new permission for an external commitment. Do not ask the owner to approve a routine technical retry.\n\n${params.task}`
        : params.task;
      const events = await this.run(bundle, prompt, signal, false, false, params.budget.max_turns);
      return { session_id: manager.getSessionId(), output: boundedOutput(bundle.current.deferredInput?.prompt ?? events.text()).text };
    } finally {
      await this.release(bundle);
    }
  }

  async compact(keepRecentTokens: number | undefined, signal: AbortSignal, channel = OWNER_CHANNEL): Promise<JsonObject> {
    return this.useChannel(channel, bundle => this.compactChannel(bundle, keepRecentTokens, signal));
  }

  private async compactChannel(bundle: SessionBundle, keepRecentTokens: number | undefined, signal: AbortSignal): Promise<JsonObject> {
    if (!bundle.session.isIdle) throw new Error("Context compaction requires an idle conversation channel");
    await this.select(bundle, this.config.routes.chat, "text", signal);
    assertActive(signal);
    const settings = bundle.session.settingsManager;
    const previous = settings.getCompactionSettings();
    if (keepRecentTokens !== undefined) settings.applyOverrides({
      compaction: { ...previous, keepRecentTokens },
    });
    const cancel = (): void => bundle.session.abortCompaction();
    signal.addEventListener("abort", cancel, { once: true });
    try {
      const result = await bundle.session.compact(
        "This is BoBe's companion conversation. Preserve the person's stated goals, preferences, boundaries, unresolved commitments, verified tool outcomes and interrupted work. Retain uncertainty. Durable memory and goals remain owned by Rust; a summary does not mutate them.",
      );
      assertActive(signal);
      await flushSession(bundle.session.sessionManager);
      return {
        session_id: bundle.session.sessionManager.getSessionId(),
        tokens_before: result.tokensBefore,
        estimated_tokens_after: result.estimatedTokensAfter ?? null,
        first_kept_entry_id: result.firstKeptEntryId,
        summary: result.summary,
      };
    } finally {
      signal.removeEventListener("abort", cancel);
      settings.applyOverrides({ compaction: previous });
    }
  }

  private async delegate(parent: SessionBundle, task: DelegateTask, signal?: AbortSignal): Promise<DelegateResult> {
    assertActive(signal);
    if (parent.current.depth >= 2) throw new Error("Scoped delegation depth is limited to two");
    if (parent.current.childrenCreated.count >= 16) throw new Error("This turn reached its scoped subagent budget");
    parent.current.childrenCreated.count++;
    const model = parent.session.model;
    if (!model) throw new Error("Parent session has no selected model");
    const controller = new AbortController();
    const budget = this.models.execution.taskBudget(task.budget, 600, 32);
    const deadline = setTimeout(() => controller.abort(new DOMException("Scoped task exceeded its chosen deadline", "TimeoutError")), budget.timeout_seconds * 1000);
    const combined = signal ? AbortSignal.any([signal, controller.signal]) : controller.signal;
    let child: SessionBundle | undefined;
    const agent = task.agent ?? "worker";
    try {
      const manager = await createChildSession(this.agentDir, this.config.workspace, parent.session.sessionManager);
      child = await this.create(manager, true, model);
      child.current = {
        ...parent.current,
        context: `Scoped delegated work. Role: ${inertMarkers(agent)}. Return findings to the parent, not to the person.\n${inertMarkers(task.context ?? "")}`,
        depth: parent.current.depth + 1,
        visibleOutput: false,
      };
      child.session.setThinkingLevel(parent.session.thinkingLevel, { persist: false });
      const result = await this.run(child, inertMarkers(task.task), combined, false, false, budget.max_turns);
      return {
        agent, ...boundedOutput(result.text()), output_tokens: result.last?.usage.output ?? 0,
        success: true, error: null,
      };
    } catch (error) {
      if (signal?.aborted) throw new Cancelled("Scoped subagent canceled");
      return {
        agent, text: "", output_tokens: 0, success: false,
        error: this.redactor.message(controller.signal.reason ?? error), truncated: false,
      };
    } finally {
      clearTimeout(deadline);
      if (child) await this.release(child);
    }
  }

  async release(bundle: SessionBundle): Promise<void> {
    try {
      await bundle.close();
    } catch (error) {
      this.callbacks.cleanupFailed(bundle.current.requestId, error);
      throw error;
    } finally {
      this.bundles.delete(bundle);
    }
  }

  async close(): Promise<void> {
    this.closed = true;
    await Promise.allSettled([...this.bundles].map(bundle => bundle.session.abort()));
    await Promise.allSettled(this.activeRuns);
    const results = await Promise.allSettled([...this.bundles].map(bundle => this.release(bundle)));
    const failures = results.filter(result => result.status === "rejected");
    this.channels.clear();
    this.channelAccess.clear();
    if (failures.length) throw new AggregateError(failures.map(failure => failure.reason), "Session shutdown failed");
  }
}
