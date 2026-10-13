import { randomUUID } from "node:crypto";
import type { BashOperations } from "@earendil-works/pi-coding-agent";
import {
  parseJsonRpcMessage, type JsonRpcMessage, type McpTransport,
  type McpTransportMessageListener, type McpTransportErrorListener, type McpTransportCloseListener,
} from "@earendil-works/pi-mcp";
import { assertActive, Cancelled } from "./errors.js";
import { isRecord, type Json, type JsonObject } from "./protocol.js";

interface ProcessState {
  output(stream: "stdout" | "stderr", bytes: Buffer): void;
  resolve(code: number): void;
  reject(error: Error): void;
  completion: Promise<number>;
}

export interface ProcessSpec {
  program: string;
  args: string[];
  cwd: string;
  env: Record<string, string>;
  purpose: "shell" | "mcp";
  request_id?: number;
}

export class ProcessClient {
  private readonly active = new Map<string, ProcessState>();
  private available = true;

  constructor(private readonly call: (method: string, params: JsonObject) => Promise<Json>) {}

  event(params: JsonObject): void {
    const id = params.process_id;
    if (typeof id !== "string") throw new Error("Process event has no identity");
    const state = this.active.get(id);
    if (!state) return;
    if (params.event === "exit") {
      this.active.delete(id);
      if (typeof params.error === "string") state.reject(new Error(params.error));
      else if (typeof params.exit_code === "number") state.resolve(params.exit_code);
      else state.reject(new Error("Process exit has no status"));
    } else if ((params.event === "stdout" || params.event === "stderr") && typeof params.data === "string") {
      state.output(params.event, Buffer.from(params.data, "base64"));
    } else {
      throw new Error("Invalid process event");
    }
  }

  async start(spec: ProcessSpec, output: ProcessState["output"]): Promise<OwnedProcess> {
    if (!this.available) throw new Cancelled("Process owner is unavailable");
    const id = randomUUID();
    let resolve: (code: number) => void = () => {};
    let reject: (error: Error) => void = () => {};
    const completion = new Promise<number>((yes, no) => { resolve = yes; reject = no; });
    void completion.catch(() => {});
    this.active.set(id, { output, resolve, reject, completion });
    const process = new OwnedProcess(id, completion, this);
    try {
      const result = await this.call("process", { action: "start", process_id: id, ...spec });
      if (!isRecord(result) || result.process_id !== id) throw new Error("Process launch identity mismatch");
      return process;
    } catch (error) {
      this.active.delete(id);
      reject(error instanceof Error ? error : new Error("Process launch failed"));
      throw error;
    }
  }

  async input(id: string, bytes: Buffer, end = false): Promise<void> {
    if (!this.available) throw new Cancelled("Process owner is unavailable");
    for (let offset = 0; offset < bytes.length || (end && offset === 0); offset += 1024 * 1024) {
      const chunk = bytes.subarray(offset, offset + 1024 * 1024);
      await this.call("process", {
        action: "write", process_id: id, data: chunk.toString("base64"),
        end: end && offset + chunk.length >= bytes.length,
      });
      if (bytes.length === 0) break;
    }
  }

  async stop(id: string): Promise<void> {
    if (!this.available || !this.active.has(id)) return;
    await this.call("process", { action: "stop", process_id: id });
  }

  disconnect(): void {
    this.available = false;
    for (const state of this.active.values()) state.reject(new Cancelled("Process owner disconnected"));
    this.active.clear();
  }

  shell(requestId: () => number): BashOperations {
    return {
      exec: async (command, cwd, options) => {
        assertActive(options.signal);
        const environment = Object.fromEntries(Object.entries(options.env ?? process.env)
          .flatMap(([key, value]) => value === undefined ? [] : [[key, value]]));
        const child = await this.start({
          purpose: "shell", request_id: requestId(), program: "/bin/bash", args: ["-c", command],
          cwd, env: environment,
        }, (_stream, data) => options.onData(data));
        let timedOut = false;
        let cancellation: Promise<void> | undefined;
        const stop = (): void => {
          cancellation ??= child.stop();
          void cancellation.catch(() => {});
        };
        const timer = options.timeout === undefined ? undefined : setTimeout(() => {
          timedOut = true;
          stop();
        }, options.timeout * 1000);
        options.signal?.addEventListener("abort", stop, { once: true });
        if (options.signal?.aborted) stop();
        try {
          const exitCode = await child.completion;
          await cancellation;
          assertActive(options.signal);
          if (timedOut) throw new Error(`timeout:${options.timeout}`);
          return { exitCode };
        } finally {
          if (timer) clearTimeout(timer);
          options.signal?.removeEventListener("abort", stop);
          await child.stop();
        }
      },
    };
  }
}

class OwnedProcess {
  constructor(readonly id: string, readonly completion: Promise<number>, private readonly owner: ProcessClient) {}
  write(bytes: Buffer): Promise<void> { return this.owner.input(this.id, bytes); }
  stop(): Promise<void> { return this.owner.stop(this.id); }
}

export class OwnedMcpTransport implements McpTransport {
  private process: OwnedProcess | undefined;
  private buffer = Buffer.alloc(0);
  private stderr = Buffer.alloc(0);
  private started = false;
  private closed = false;
  private closing = false;
  private closePromise: Promise<void> | undefined;
  private readonly messages = new Set<McpTransportMessageListener>();
  private readonly errors = new Set<McpTransportErrorListener>();
  private readonly closes = new Set<McpTransportCloseListener>();

  constructor(private readonly owner: ProcessClient, private readonly spec: ProcessSpec) {}

  async start(): Promise<void> {
    if (this.started || this.closed) throw new Error("MCP transport cannot be restarted");
    this.started = true;
    this.process = await this.owner.start(this.spec, (stream, data) => {
      if (stream === "stdout") this.receive(data);
      else this.stderr = Buffer.concat([this.stderr, data]).subarray(-64 * 1024);
    });
    void this.process.completion.then(
      code => {
        if (!this.closing && !this.closed && code !== 0) this.error(new Error(`MCP process exited with code ${code}: ${this.stderr.toString("utf8")}`));
        this.finish();
      },
      error => { this.error(error); this.finish(); },
    );
  }

  async send(message: JsonRpcMessage): Promise<void> {
    if (!this.process || this.closed) throw new Error("MCP process is not available");
    await this.process.write(Buffer.from(`${JSON.stringify(message)}\n`));
  }

  close(): Promise<void> {
    this.closePromise ??= this.stop();
    return this.closePromise;
  }

  private async stop(): Promise<void> {
    if (this.closed) return;
    this.closing = true;
    if (this.process) {
      await this.process.stop();
      await this.process.completion.catch(error => this.error(error));
    }
    this.finish();
  }

  private receive(data: Buffer): void {
    this.buffer = Buffer.concat([this.buffer, data]);
    for (;;) {
      const newline = this.buffer.indexOf(10);
      if (newline < 0) {
        if (this.buffer.length > 16 * 1024 * 1024) {
          this.buffer = Buffer.alloc(0);
          this.error(new Error("MCP output exceeds the message limit"));
          void this.close().catch(error => this.error(error));
        }
        return;
      }
      const line = this.buffer.subarray(0, newline);
      this.buffer = this.buffer.subarray(newline + 1);
      if (!line.toString("utf8").trim()) continue;
      try {
        if (line.length > 16 * 1024 * 1024) throw new Error("MCP output exceeds the message limit");
        const message = parseJsonRpcMessage(JSON.parse(line.toString("utf8")));
        for (const listener of this.messages) listener(message);
      } catch (error) { this.error(error); }
    }
  }

  private error(error: unknown): void {
    const failure = error instanceof Error ? error : new Error("MCP process failed");
    for (const listener of this.errors) listener(failure);
  }

  private finish(): void {
    if (this.closed) return;
    this.closed = true;
    if (this.buffer.toString("utf8").trim()) this.error(new Error("MCP closed with an incomplete message"));
    this.buffer = Buffer.alloc(0);
    for (const listener of this.closes) listener();
  }

  onMessage(listener: McpTransportMessageListener): () => void { this.messages.add(listener); return () => this.messages.delete(listener); }
  onError(listener: McpTransportErrorListener): () => void { this.errors.add(listener); return () => this.errors.delete(listener); }
  onClose(listener: McpTransportCloseListener): () => void { this.closes.add(listener); return () => this.closes.delete(listener); }
}
