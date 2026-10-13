import { join } from "node:path";
import {
  createMcpExtension, defineTool, type ExtensionFactory, type McpServerEntry, type ToolDefinition,
} from "@earendil-works/pi-coding-agent";
import {
  type JsonRpcMessage, type McpTransport,
  type McpTransportCloseListener, type McpTransportErrorListener, type McpTransportMessageListener,
} from "@earendil-works/pi-mcp";
import { assertActive, abortableDelay, type Redactor } from "./errors.js";
import { isRecord, type Json, type McpConfig } from "./protocol.js";
import { OwnedMcpTransport, type ProcessClient } from "./process.js";

interface ServerStatus {
  name: string;
  connected: boolean;
  status: "starting" | "connecting" | "connected" | "disconnected" | "failed" | "closed";
  error: string | null;
  tools: { name: string; description: string }[];
}

class ObservedTransport implements McpTransport {
  private readonly pending = new Map<string | number, { method: string; replace: boolean }>();

  constructor(
    private readonly transport: McpTransport,
    private readonly state: ServerStatus,
    private readonly redactor: Redactor,
  ) {
    transport.onMessage(message => this.observe(message));
    transport.onError(error => {
      state.connected = false;
      state.status = "failed";
      state.error = redactor.message(error);
    });
    transport.onClose(() => {
      state.connected = false;
      if (state.status !== "failed" && state.status !== "closed") state.status = "disconnected";
    });
  }

  async start(): Promise<void> {
    this.state.status = "connecting";
    try {
      await this.transport.start();
    } catch (error) {
      this.state.status = "failed";
      this.state.error = this.redactor.message(error);
      throw error;
    }
  }

  async send(message: JsonRpcMessage): Promise<void> {
    if ("method" in message && "id" in message) {
      this.pending.set(message.id, {
        method: message.method,
        replace: message.method === "tools/list" && (!isRecord(message.params) || !message.params.cursor),
      });
    }
    await this.transport.send(message);
  }

  private observe(message: JsonRpcMessage): void {
    if (!("id" in message) || "method" in message || message.id === null) return;
    const pending = this.pending.get(message.id);
    const method = pending?.method;
    this.pending.delete(message.id);
    if (method !== "initialize" && method !== "tools/list") return;
    if ("error" in message) {
      this.state.status = "failed";
      this.state.connected = false;
      this.state.error = this.redactor.message(new Error(message.error.message));
      return;
    }
    if (method === "initialize") {
      const result = message.result;
      if (isRecord(result) && isRecord(result.capabilities) && !("tools" in result.capabilities)) {
        this.state.status = "connected";
        this.state.connected = true;
        this.state.error = null;
      }
    } else {
      if (!isRecord(message.result) || !Array.isArray(message.result.tools)) {
        this.state.status = "failed";
        this.state.error = "MCP returned an invalid tool catalog";
        return;
      }
      const tools: ServerStatus["tools"] = [];
      for (const tool of message.result.tools) {
        if (!isRecord(tool) || typeof tool.name !== "string") {
          this.state.status = "failed";
          this.state.error = "MCP returned an invalid tool definition";
          return;
        }
        tools.push({ name: tool.name, description: typeof tool.description === "string" ? tool.description : "" });
      }
      this.state.tools = [...new Map([
        ...(pending?.replace ? [] : this.state.tools),
        ...tools,
      ].map(tool => [tool.name, tool])).values()];
      const more = typeof message.result.nextCursor === "string" && message.result.nextCursor.length > 0;
      this.state.connected = !more;
      this.state.status = more ? "connecting" : "connected";
      this.state.error = null;
    }
  }

  async close(): Promise<void> {
    this.state.status = "closed";
    this.state.connected = false;
    await this.transport.close();
  }

  hasActiveRequest(): boolean {
    return [...this.pending.values()].some(({ method }) => method === "tools/call" || method.startsWith("resources/"));
  }

  onMessage(listener: McpTransportMessageListener): () => void { return this.transport.onMessage(listener); }
  onError(listener: McpTransportErrorListener): () => void { return this.transport.onError(listener); }
  onClose(listener: McpTransportCloseListener): () => void { return this.transport.onClose(listener); }
}

export class McpResources {
  private readonly states = new Map<string, ServerStatus>();
  private readonly transports = new Set<ObservedTransport>();
  private restartRequired = false;

  constructor(private config: McpConfig, private readonly agentDir: string, private readonly redactor: Redactor,
    private readonly processes: ProcessClient, private readonly guardTool: (tool: ToolDefinition) => ToolDefinition) {
    this.reset(config);
  }

  private reset(config: McpConfig): void {
    const names = new Set<string>();
    for (const name of Object.keys(config.servers)) {
      const namespace = name.replaceAll("-", "_");
      if (names.has(namespace)) throw new Error("MCP server names collide after pi namespace normalization");
      names.add(namespace);
    }
    this.config = config;
    this.states.clear();
    for (const [name, server] of Object.entries(config.servers)) {
      for (const [key, value] of Object.entries(server.env)) {
        if (/SECRET|TOKEN|PASSWORD|API_KEY$/i.test(key)) this.redactor.add(value);
      }
      this.states.set(name, { name, connected: false, status: "starting", error: null, tools: [] });
    }
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
          env: { ...Object.fromEntries(Object.entries(process.env).flatMap(([key, value]) =>
            value === undefined ? [] : [[key, value]])), ...entry.config.env },
        }), state, this.redactor);
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
    await this.close();
    this.reset(config);
    this.restartRequired = false;
  }

  async cancelActive(): Promise<void> {
    const active = [...this.transports].filter(transport => transport.hasActiveRequest());
    if (active.length === 0) return;
    this.restartRequired = true;
    const outcomes = await Promise.allSettled(active.map(transport => transport.close()));
    const failures = outcomes.filter(outcome => outcome.status === "rejected");
    if (failures.length) throw new AggregateError(failures.map(failure => failure.reason), "MCP cancellation failed");
  }

  needsRestart(): boolean {
    return this.restartRequired || [...this.states.values()].some(state =>
      state.status === "disconnected" || state.status === "closed");
  }

  async prepareRestart(): Promise<void> {
    await this.replace(this.config);
  }

  status(): Json {
    const wireStatus = {
      starting: "pending", connecting: "pending", connected: "connected",
      disconnected: "stopped", failed: "failed", closed: "stopped",
    } satisfies Record<ServerStatus["status"], string>;
    return [...this.states.values()].map(({ name, connected, status, error }) =>
      ({ name, connected, status: wireStatus[status], error }));
  }

  tools(name: string): Json {
    const state = this.states.get(name);
    const config = this.config.servers[name];
    if (!state || !config) throw new Error("Unknown MCP server");
    if (!state.connected) throw new Error(`MCP server ${name} is ${state.status}`);
    return state.tools.filter(tool => !config.excluded_tools.some(pattern =>
      new RegExp(`^${pattern.split("*").map(part => part.replace(/[.+?^${}()|[\]\\]/g, "\\$&")).join(".*")}$`).test(tool.name)));
  }

  async ready(signal?: AbortSignal): Promise<void> {
    const deadline = Date.now() + Math.max(20_000, ...Object.values(this.config.servers).map(server => server.timeout_ms));
    while ([...this.states.values()].some(state => !state.connected)) {
      assertActive(signal);
      const failed = [...this.states.values()].find(state =>
        ["failed", "closed", "disconnected"].includes(state.status));
      if (failed) throw new Error(`MCP server ${failed.name}: ${failed.error ?? failed.status}`);
      if (Date.now() >= deadline) throw new Error("MCP startup timed out");
      await abortableDelay(10, signal);
    }
  }

  async close(): Promise<void> {
    const outcomes = await Promise.allSettled([...this.transports].map(transport => transport.close()));
    const failures = outcomes.filter(outcome => outcome.status === "rejected");
    if (failures.length) throw new AggregateError(failures.map(failure => failure.reason), "MCP shutdown failed");
    this.transports.clear();
  }
}
