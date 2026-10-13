import { JsonlRpc } from "./rpc.js";
import { Cancelled, Redactor } from "./errors.js";
import { delimiter, isAbsolute, join } from "node:path";
import { fileURLToPath } from "node:url";
import { emptySchema, initializeSchema, validate, type Json, type Request } from "./protocol.js";
import type { WorkerHost } from "./worker.js";

const safeEnvironment = new Set([
  "PATH", "HOME", "USER", "LOGNAME", "SHELL", "LANG", "LANGUAGE", "LC_ALL", "LC_CTYPE",
  "TZ", "TERM", "COLORTERM", "SYSTEMROOT", "SystemRoot", "WINDIR", "COMSPEC", "PATHEXT",
  "SSL_CERT_FILE", "SSL_CERT_DIR", "NODE_EXTRA_CA_CERTS", "HTTP_PROXY", "HTTPS_PROXY",
  "ALL_PROXY", "NO_PROXY", "http_proxy", "https_proxy", "all_proxy", "no_proxy",
]);
for (const name of Object.keys(process.env)) {
  if (!safeEnvironment.has(name)) delete process.env[name];
}
process.umask(0o077);

const redactor = new Redactor();
const rpc = new JsonlRpc(process.stdin, process.stdout, redactor);
let loading: Promise<WorkerHost> | undefined;
let loaded: WorkerHost | undefined;
let closing = false;
const bootstrap = {
  notification: (method: string, params: import("./protocol.js").JsonObject): void => {
    if (!loaded) throw new Error("Process event before worker initialization");
    loaded.notification(method, params);
  },
  request: async (request: Request): Promise<Json> => {
    if (closing) throw new Cancelled("Worker is shutting down");
    if (!loading) {
      if (request.method === "shutdown") {
        validate(emptySchema, request.params, "shutdown parameters");
        closing = true;
        return {};
      }
      if (request.method !== "initialize") throw new Error("Worker is not initialized");
      const config = validate(initializeSchema, request.params, "initialize parameters");
      if (!isAbsolute(config.data_root)) throw new Error("data_root must be absolute");
      process.env.PI_CODING_AGENT_DIR = join(config.data_root, "agent");
      process.env.PI_OFFLINE = "1";
      process.env.PI_SKIP_VERSION_CHECK = "1";
      const packagedBin = fileURLToPath(new URL("../bin/", import.meta.url));
      process.env.PATH = [packagedBin, process.env.PATH].filter(value => value !== undefined).join(delimiter);
      loading = import("./worker.js").then(({ WorkerHost }) => {
        loaded = new WorkerHost(rpc);
        return loaded;
      });
    }
    const host = await loading;
    if (closing) throw new Cancelled("Worker is shutting down");
    return host.request(request);
  },
  close: async (): Promise<void> => {
    closing = true;
    const host = await loading;
    host?.disconnect();
    await host?.close();
  },
};
let failed = false;
const report = (error: unknown): void => {
  if (failed) return;
  failed = true;
  process.stderr.write(`BoBe pi worker failed: ${redactor.message(error)}\n`);
  process.exitCode = 1;
};

try {
  const [major, minor] = process.versions.node.split(".").map(Number);
  if (major === undefined || minor === undefined || major < 22 || (major === 22 && minor < 19)) {
    throw new Error("BoBe pi requires Node.js 22.19.0 or later");
  }
  const stop = (): void => { void rpc.finish().catch(report); };
  process.once("SIGTERM", stop);
  process.once("SIGINT", stop);
  try {
    await rpc.start(bootstrap);
    await rpc.flush();
  } finally {
    process.removeListener("SIGTERM", stop);
    process.removeListener("SIGINT", stop);
    await bootstrap.close();
  }
} catch (error) {
  report(error);
}
