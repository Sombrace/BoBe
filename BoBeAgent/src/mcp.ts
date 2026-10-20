import { join } from "node:path";
import {
  createMcpExtension, defineTool, type ExtensionFactory, type McpServerEntry, type ToolDefinition,
} from "@earendil-works/pi-coding-agent";
import type {
  JsonRpcMessage, McpTransport, McpTransportSendOptions,
  McpTransportCloseListener, McpTransportErrorListener, McpTransportMessageListener,
} from "@earendil-works/pi-mcp";
import { assertActive, onAbort, rejections, type Redactor } from "./errors.ts";
import { isRecord, type Json, type McpConfig } from "./protocol.ts";
import { OwnedMcpTransport, environment, type ProcessClient } from "./process.ts";

interface ServerStatus {
  name: string;
  connected: boolean;
  status: "starting" | "connecting" | "connected" | "disconnected" | "failed" | "closed";
  error: string | null;
  tools: { name: string; description: string }[];
}

class ObservedTransport implements McpTransport {
  private readonly pending = new Map<string | number, { method: string; replace: boolean }>();
  private readonly transport: McpTransport;
  private readonly state: ServerStatus;
  private readonly redactor: Redactor;
  private readonly changed: () => void;

  constructor(transport: McpTransport, state: ServerStatus, redactor: Redactor, changed: () => void) {
    this.transport = transport;
    this.state = state;
    this.redactor = redactor;
    this.changed = changed;
    transport.onMessage(message => this.observe(message));
    transport.onError(error => this.update({ connected: false, status: "failed", error: redactor.message(error) }));
    transport.onClose(() => this.update({
      connected: false,
      ...(state.status !== "failed" && state.status !== "closed" ? { status: "disconnected" } : {}),
    }));
  }

  private update(change: Partial<ServerStatus>): void {
    Object.assign(this.state, change);
    this.changed();
  }

  async start(): Promise<void> {
    this.update({ status: "connecting" });
    try {
      await this.transport.start();
    } catch (error) {
      this.update({ status: "failed", error: this.redactor.message(error) });
      throw error;
    }
  }

  async send(message: JsonRpcMessage, options?: McpTransportSendOptions): Promise<void> {
    const id = "method" in message && "id" in message ? message.id : undefined;
    if (id !== undefined && "method" in message) {
      this.pending.set(id, {
        method: message.method,
        replace: message.method === "tools/list" && (!isRecord(message.params) || !message.params.cursor),
      });
    }
    try {
      await this.transport.send(message, options);
    } catch (error) {
      if (id !== undefined) this.pending.delete(id);
      throw error;
    }
  }

  private observe(message: JsonRpcMessage): void {
    if (!("id" in message) || "method" in message || message.id === null) return;
    const pending = this.pending.get(message.id);
    const method = pending?.method;
    this.pending.delete(message.id);
    if (method !== "initialize" && method !== "tools/list") return;
    if ("error" in message) {
      this.update({ status: "failed", connected: false, error: this.redactor.message(new Error(message.error.message)) });
      return;
    }
    if (method === "initialize") {
      const result = message.result;
      if (isRecord(result) && isRecord(result.capabilities) && !("tools" in result.capabilities)) {
        this.update({ status: "connected", connected: true, error: null });
      }
      return;
    }
    if (!isRecord(message.result) || !Array.isArray(message.result.tools)) {
      this.update({ status: "failed", error: "MCP returned an invalid tool catalog" });
      return;
    }
    const tools: ServerStatus["tools"] = [];
    for (const tool of message.result.tools) {
      if (!isRecord(tool) || typeof tool.name !== "string") {
        this.update({ status: "failed", error: "MCP returned an invalid tool definition" });
        return;
      }
      tools.push({ name: tool.name, description: typeof tool.description === "string" ? tool.description : "" });
    }
    const more = typeof message.result.nextCursor === "string" && message.result.nextCursor.length > 0;
    this.update({
      tools: [...new Map([...(pending?.replace ? [] : this.state.tools), ...tools].map(tool => [tool.name, tool])).values()],
      connected: !more,
      status: more ? "connecting" : "connected",
      error: null,
    });
  }

  async close(): Promise<void> {
    this.update({ status: "closed", connected: false });
    await this.transport.close();
  }

  hasActiveRequest(): boolean {
    return this.pending.values().some(({ method }) => method === "tools/call" || method.startsWith("resources/"));
  }

  onMessage(listener: McpTransportMessageListener): () => void { return this.transport.onMessage(listener); }
  onError(listener: McpTransportErrorListener): () => void { return this.transport.onError(listener); }
  onClose(listener: McpTransportCloseListener): () => void { return this.transport.onClose(listener); }
}

function checkServerNames(config: McpConfig): void {
  const names = new Set<string>();
  for (const name of Object.keys(config.servers)) {
    const namespace = name.replaceAll("-", "_");
    if (names.has(namespace)) throw new Error("MCP server names collide after pi namespace normalization");
    names.add(namespace);
  }
}

export class McpResources {
  private readonly states = new Map<string, ServerStatus>();
  private readonly transports = new Set<ObservedTransport>();
  private restartRequired = false;
  private changes = Promise.withResolvers<void>();

  private config: McpConfig;
  private readonly agentDir: string;
  private readonly redactor: Redactor;
  private readonly processes: ProcessClient;
  private readonly guardTool: (tool: ToolDefinition) => ToolDefinition;

  constructor(
    config: McpConfig, agentDir: string, redactor: Redactor, processes: ProcessClient,
    guardTool: (tool: ToolDefinition) => ToolDefinition,
  ) {
    this.config = config;
    this.agentDir = agentDir;
    this.redactor = redactor;
    this.processes = processes;
    this.guardTool = guardTool;
    this.reset(config);
  }

  private reset(config: McpConfig): void {
    checkServerNames(config);
    this.config = config;
    this.states.clear();
    for (const [name, server] of Object.entries(config.servers)) {
      for (const [key, value] of Object.entries(server.env)) {
        if (/SECRET|TOKEN|PASSWORD|API_KEY$/i.test(key)) this.redactor.add(value);
      }
      this.states.set(name, { name, connected: false, status: "starting", error: null, tools: [] });
    }
    this.changed();
  }

  private changed(): void {
    this.changes.resolve();
    this.changes = Promise.withResolvers();
  }

  extension(): ExtensionFactory {
    const extension = createMcpExtension({
      loadConfig: () => ({
        servers: Object.entries(this.config.servers).map(([name, server]): McpServerEntry => ({
          name,
          source: "<bobe-owner>",
          scope: "extension",
          config: {
            type: "stdio", command: server.command, args: server.args, env: server.env,
            timeout: server.timeout_ms / 1_000,
            exposure: "direct",
            toolExposure: Object.fromEntries(server.excluded_tools.map(tool => [tool, "hidden"])),
          },
        })),
        errors: [],
        autoEnableCodemode: false,
      }),
      createTransport: (entry, cwd) => {
        const state = this.states.get(entry.name);
        if (!state || !("command" in entry.config)) throw new Error("Unknown owner MCP server");
        const transport = new ObservedTransport(new OwnedMcpTransport(this.processes, {
          program: entry.config.command,
          args: entry.config.args ?? [],
          cwd,
          purpose: "mcp",
          env: { ...environment(), ...entry.config.env },
        }), state, this.redactor, () => this.changed());
        this.transports.add(transport);
        transport.onClose(() => this.transports.delete(transport));
        return transport;
      },
      logPath: join(this.agentDir, "mcp.log"),
      openUrl: () => { throw new Error("MCP sign-in is not an interactive worker operation"); },
      updateConfig: () => { throw new Error("MCP configuration is owned by the daemon"); },
      startupWaitMs: 20_000,
    });
    return api => extension({
      ...api,
      registerTool: tool => api.registerTool(this.guardTool(defineTool(tool))),
    });
  }

  async replace(config: McpConfig): Promise<void> {
    checkServerNames(config);
    let stopped = false;
    try {
      await this.close();
      stopped = true;
    } finally {
      this.reset(config);
      this.restartRequired = !stopped;
    }
  }

  async cancelActive(): Promise<void> {
    const active = this.transports.values().filter(transport => transport.hasActiveRequest()).toArray();
    if (active.length === 0) return;
    this.restartRequired = true;
    const failures = rejections(await Promise.allSettled(active.map(transport => transport.close())));
    if (failures.length) throw new AggregateError(failures, "MCP cancellation failed");
  }

  needsRestart(): boolean {
    return this.restartRequired || this.states.values().some(state =>
      state.status === "disconnected" || state.status === "closed" || state.status === "failed");
  }

  async prepareRestart(): Promise<void> {
    await this.replace(this.config);
  }

  status(): Json {
    const wireStatus = {
      starting: "pending", connecting: "pending", connected: "connected",
      disconnected: "stopped", failed: "failed", closed: "stopped",
    } satisfies Record<ServerStatus["status"], string>;
    return this.states.values().map(({ name, connected, status, error }) =>
      ({ name, connected, status: wireStatus[status], error })).toArray();
  }

  tools(name: string): Json {
    const state = this.states.get(name);
    const config = this.config.servers[name];
    if (!state || !config) throw new Error("Unknown MCP server");
    if (!state.connected) throw new Error(`MCP server ${name} is ${state.status}`);
    const excluded = config.excluded_tools.map(pattern =>
      new RegExp(`^${pattern.split("*").map(part => RegExp.escape(part)).join(".*")}$`));
    return state.tools.filter(tool => !excluded.some(pattern => pattern.test(tool.name)));
  }

  async ready(signal?: AbortSignal): Promise<void> {
    const deadline = AbortSignal.timeout(Math.max(20_000, ...Object.values(this.config.servers).map(server => server.timeout_ms)));
    const wake = signal ? AbortSignal.any([signal, deadline]) : deadline;
    while (this.states.values().some(state => !state.connected)) {
      assertActive(signal);
      const failed = this.states.values().find(state => ["failed", "closed", "disconnected"].includes(state.status));
      if (failed) throw new Error(`MCP server ${failed.name}: ${failed.error ?? failed.status}`);
      if (deadline.aborted) throw new Error("MCP startup timed out");
      const aborted = Promise.withResolvers<void>();
      using _abort = onAbort(wake, aborted.resolve);
      await Promise.race([this.changes.promise, aborted.promise]);
    }
  }

  async close(): Promise<void> {
    const failures = rejections(await Promise.allSettled(Array.from(this.transports, transport => transport.close())));
    if (failures.length) throw new AggregateError(failures, "MCP shutdown failed");
    this.transports.clear();
  }
}
