import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { once } from "node:events";
import { object, type Json, type JsonObject } from "./protocol.ts";

export class ProcessFixture {
  private readonly processes = new Map<string, { child: ChildProcessWithoutNullStreams; closed: Promise<void> }>();
  readonly programs:string[]=[];
  failingStops = 0;

  private readonly notify: (params: JsonObject) => void;
  private readonly capacity: number;

  constructor(notify: (params: JsonObject) => void, capacity = 64) {
    this.notify = notify;
    this.capacity = capacity;
  }

  async invoke(params: JsonObject): Promise<Json> {
    const id = params.process_id;
    if (typeof id !== "string") throw new Error("Fixture process identity missing");
    if (params.action === "start") {
      if (this.processes.size>=this.capacity) throw new Error("owned process capacity is full");
      if (typeof params.program !== "string" || typeof params.cwd !== "string" ||
        !Array.isArray(params.args) || !params.args.every(arg => typeof arg === "string")) {
        throw new Error("Fixture process input invalid");
      }
      const env = Object.fromEntries(Object.entries(object(params.env)).map(([key, value]) => {
        if (typeof value !== "string") throw new Error("Fixture environment value invalid");
        return [key, value];
      }));
      this.programs.push(params.program);
      const child = spawn(params.program, params.args, { cwd: params.cwd, env, detached: true, stdio: "pipe" });
      const closed = new Promise<void>(resolve => {
        child.once("close", code => {
          this.notify({ process_id: id, event: "exit", exit_code: code ?? 143, error: null });
          this.processes.delete(id);
          resolve();
        });
      });
      this.processes.set(id, { child, closed });
      child.stdout.on("data", (data: Buffer) => this.notify({ process_id: id, event: "stdout", data: data.toBase64() }));
      child.stderr.on("data", (data: Buffer) => this.notify({ process_id: id, event: "stderr", data: data.toBase64() }));
      child.stdin.on("error", () => {});
      try { await once(child, "spawn"); }
      catch (error) { this.processes.delete(id); throw error; }
      return { process_id: id };
    }
    const active = this.processes.get(id);
    if (params.action === "stop") {
      if (this.failingStops > 0) {
        this.failingStops--;
        throw new Error("Injected process stop failure");
      }
      if (active) {
        this.kill(active.child);
        await active.closed;
      }
      return {};
    }
    if (params.action === "write") {
      if (!active || typeof params.data !== "string") throw new Error("Fixture process is not running");
      const bytes = Buffer.from(params.data, "base64");
      await new Promise<void>((resolve, reject) => active.child.stdin.write(bytes, error => error ? reject(error) : resolve()));
      if (params.end) active.child.stdin.end();
      return {};
    }
    throw new Error("Unknown fixture process operation");
  }

  private kill(child: ChildProcessWithoutNullStreams): void {
    if (!child.pid) return;
    try { process.kill(-child.pid, "SIGKILL"); }
    catch (error) {
      const code = Error.isError(error) && "code" in error ? error.code : undefined;
      if (code !== "ESRCH" && code !== "EPERM") throw error;
    }
  }

  async close(): Promise<void> {
    const active = [...this.processes.values()];
    for (const entry of active) this.kill(entry.child);
    await Promise.all(active.map(entry => entry.closed));
  }
}
