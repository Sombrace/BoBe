import type { Readable, Writable } from "node:stream";
import { Cancelled, Redactor, WorkerFault, asError, assertActive } from "./errors.ts";
import { MAX_LINE_BYTES, json, object, isRecord, parseEnvelope, type Json, type JsonObject, type Notification, type Request } from "./protocol.ts";

interface Pending {
  resolve(value: Json): void;
  reject(error: Error): void;
}

export interface RpcHandler {
  request(request: Request): Promise<Json>;
  close(error?: Error): Promise<void>;
  notification?(method: string, params: JsonObject): void;
}

export class JsonlRpc {
  private nextId = -1;
  private readonly callbacks = new Map<number, Pending>();
  private lastRequest = 0;
  private readonly handlers = new Set<Promise<void>>();
  private writes: Promise<void> = Promise.resolve();
  private queuedWrites = 0;
  private queuedBytes = 0;
  private handler: RpcHandler | undefined;
  private pending: Buffer[] = [];
  private pendingBytes = 0;
  private closed = false;
  private ending = false;
  private completion: Promise<void> | undefined;
  private finishing: Promise<void> | undefined;
  private terminalError: Error | undefined;
  private readonly decoder = new TextDecoder("utf-8", { fatal: true });
  private readonly outputError = (error: Error): void => { this.terminate(error); };
  private readonly input: Readable;
  private readonly output: Writable;
  readonly redactor: Redactor;

  constructor(input: Readable, output: Writable, redactor = new Redactor()) {
    this.input = input;
    this.output = output;
    this.redactor = redactor;
  }

  start(handler: RpcHandler): Promise<void> {
    if (this.handler) throw new Error("RPC transport already started");
    this.handler = handler;
    this.output.on("error", this.outputError);
    this.output.once("close", () => this.output.off("error", this.outputError));
    this.completion = this.read();
    return this.completion;
  }

  notify(notification: Notification): void {
    this.enqueue(json(notification));
  }

  private enqueue(value: Json): void {
    const line = `${JSON.stringify(value)}\n`;
    if (Buffer.byteLength(line) - 1 > MAX_LINE_BYTES) throw new Error("RPC output exceeds 16 MiB");
    if (this.closed) throw new Error("RPC transport is closed");
    const bytes = Buffer.byteLength(line);
    if (this.queuedWrites >= 1024 || this.queuedBytes + bytes > MAX_LINE_BYTES * 2) {
      const error = new Error("RPC output queue capacity exceeded");
      this.terminate(error);
      throw error;
    }
    this.queuedWrites++;
    this.queuedBytes += bytes;
    this.writes = this.writes.then(() => this.write(line)).finally(() => {
      this.queuedWrites--;
      this.queuedBytes -= bytes;
    });
    void this.writes.catch(error => {
      this.terminate(asError(error, "RPC write failed"));
    });
  }

  private async write(line: string): Promise<void> {
    const { promise, resolve, reject } = Promise.withResolvers<void>();
    using _deadline = setTimeout(() => {
      const error = new Error("RPC output did not drain within 15 seconds");
      this.output.destroy(error);
      reject(error);
    }, 15_000);
    this.output.write(line, error => {
      if (error) reject(error);
      else resolve();
    });
    await promise;
  }

  async flush(): Promise<void> {
    await this.writes;
  }

  async call(method: string, params: JsonObject, signal?: AbortSignal): Promise<Json> {
    assertActive(signal);
    if (this.closed || this.ending) throw new Error("RPC transport is closing");
    if (this.callbacks.size >= 256) throw new Error("RPC callback capacity reached");
    const id = this.nextId--;
    if (!Number.isSafeInteger(id)) throw new Error("RPC callback id space exhausted");
    const { promise: response, resolve, reject } = Promise.withResolvers<Json>();
    this.callbacks.set(id, { resolve, reject });
    try {
      this.enqueue({ id, method, params });
    } catch (error) {
      this.callbacks.delete(id);
      reject(asError(error, "RPC callback could not be sent"));
    }
    const value = await response;
    assertActive(signal);
    return value;
  }

  private dispatch(value: unknown): void {
    if (isRecord(value) && !("id" in value) && value.method === "process_event"
      && Object.keys(value).every(key => key === "method" || key === "params")) {
      if (!this.handler?.notification) throw new Error("Host notifications are not supported");
      this.handler.notification(value.method, object(value.params));
      return;
    }
    const envelope = parseEnvelope(value);
    if ("method" in envelope) {
      if (this.ending) throw new Error("RPC request after shutdown");
      if (envelope.id <= this.lastRequest) throw new Error("Duplicate or out-of-order RPC request id");
      if (this.handlers.size >= 256) throw new Error("RPC request capacity reached");
      this.lastRequest = envelope.id;
      const handler = this.handler;
      if (!handler) throw new Error("RPC handler missing");
      const task = Promise.try(() => handler.request(envelope))
        .then(
          result => this.enqueue({ id: envelope.id, result: json(result) }),
          error => {
            this.enqueue({ id: envelope.id, error: { message: this.redactor.message(error) } });
            if (error instanceof WorkerFault) this.terminate(error);
          },
        );
      this.handlers.add(task);
      void task.then(
        () => {
          this.handlers.delete(task);
          if (envelope.method === "shutdown") {
            this.ending = true;
            this.observeTermination(this.finish());
          }
        },
        error => {
          this.handlers.delete(task);
          this.terminate(asError(error, "RPC dispatch failed"));
        },
      );
    } else {
      const callback = this.callbacks.get(envelope.id);
      if (!callback) throw new Error("Unknown RPC response id");
      this.callbacks.delete(envelope.id);
      if ("error" in envelope) callback.reject(new Error(envelope.error.message));
      else callback.resolve(envelope.result);
    }
  }

  private async read(): Promise<void> {
    try {
      for await (const raw of this.input) {
        let chunk = Buffer.isBuffer(raw) ? raw : Buffer.from(String(raw));
        let newline = chunk.indexOf(10);
        while (newline !== -1) {
          if (this.pendingBytes + newline > MAX_LINE_BYTES) throw new Error("RPC input exceeds 16 MiB");
          const line = Buffer.concat([...this.pending, chunk.subarray(0, newline)]);
          this.pending = [];
          this.pendingBytes = 0;
          chunk = chunk.subarray(newline + 1);
          if (line.length === 0) throw new Error("Empty RPC message");
          let parsed: unknown;
          try {
            parsed = JSON.parse(this.decoder.decode(line));
          } catch (error) {
            throw new Error("Malformed RPC JSON or UTF-8", { cause: error });
          }
          this.dispatch(parsed);
          newline = chunk.indexOf(10);
        }
        if (chunk.length) {
          this.pending.push(chunk);
          this.pendingBytes += chunk.length;
        }
        if (this.pendingBytes > MAX_LINE_BYTES) throw new Error("RPC input exceeds 16 MiB");
      }
      if (this.pendingBytes !== 0) throw new Error("Incomplete RPC message at EOF");
      if (!this.closed) await this.finish();
    } catch (error) {
      if (!this.closed) {
        this.terminate(asError(error, "RPC read failed"));
      }
    }
    await this.finishing;
    if (this.terminalError) throw this.terminalError;
  }

  private rejectCallbacks(error: Error): void {
    for (const callback of this.callbacks.values()) callback.reject(error);
    this.callbacks.clear();
  }

  terminate(error: Error): void {
    this.terminalError ??= error;
    this.observeTermination(this.finish());
  }

  private observeTermination(promise: Promise<void>): void {
    void promise.catch(error => {
      this.terminalError ??= asError(error, "RPC shutdown failed");
    });
  }

  finish(): Promise<void> {
    this.finishing ??= this.finishOnce();
    return this.finishing;
  }

  private async finishOnce(): Promise<void> {
    this.ending = true;
    this.rejectCallbacks(this.terminalError ?? new Cancelled("RPC input closed"));
    try {
      await this.handler?.close(this.terminalError);
      const settled = await Promise.allSettled(this.handlers);
      const failure = settled.find(result => result.status === "rejected");
      if (failure?.status === "rejected") throw failure.reason;
      await this.flush();
    } catch (error) {
      this.terminalError ??= asError(error, "RPC shutdown failed");
    } finally {
      this.closed = true;
      this.input.destroy();
    }
    if (this.terminalError) throw this.terminalError;
  }
}
