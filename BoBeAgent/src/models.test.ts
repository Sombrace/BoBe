import assert from "node:assert/strict";
import test from "node:test";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { once } from "node:events";
import { spawn } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { WorkerHost, initializeParams } from "./worker.js";
import { ModelProfiles } from "./models.js";
import { Redactor } from "./errors.js";
import { configuration, deferred, fixture } from "./test-support.js";
import { isRecord, object, type Notification } from "./protocol.js";
import { join } from "node:path";
import { ProcessFixture } from "./process-fixture.js";

async function listen(
  handler: (request: IncomingMessage, response: ServerResponse) => void,
): Promise<{ url: string; close(): Promise<void> }> {
  const server = createServer(handler);
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert.ok(address && typeof address !== "string");
  return {
    url: `http://127.0.0.1:${address.port}/v1`,
    close: () => new Promise<void>((resolve, reject) => {
      server.closeAllConnections();
      server.close(error => error ? reject(error) : resolve());
    }),
  };
}

test("generic catalog discovery uses the owner token and never fabricates a successful catalog", async t => {
  const paths = await fixture(t);
  let status = 200;
  const server = await listen((request, response) => {
    assert.equal(request.headers.authorization, "Bearer relay-owner-token");
    response.writeHead(status, { "content-type": "application/json" });
    response.end(status === 200 ? JSON.stringify({ data: [
      { id: "model-with-limits", name: "Known model", context_window: 16_384, max_output_tokens: 2_048, vision: true },
      { id: "id-only" },
    ] }) : '{"error":"catalog unavailable"}');
  });
  const config = configuration(paths);
  config.profiles.test = { kind: "open_ai_compatible", auth_method: "api_key", base_url: server.url, api_key: "relay-owner-token" };
  const profiles = await ModelProfiles.create(config, join(paths.root, "agent"), () => {}, new Redactor());
  try {
    const models = await profiles.models("test");
    assert.ok(Array.isArray(models));
    const known = models.find(model => isRecord(model) && model.id === "model-with-limits");
    const unknown = models.find(model => isRecord(model) && model.id === "id-only");
    assert.ok(isRecord(known) && known.context_window === 16_384 && known.vision === true);
    assert.ok(isRecord(unknown) && unknown.context_window === null && unknown.vision === false);
    status = 503;
    await assert.rejects(profiles.models("test"), /HTTP 503/);
  } finally {
    await profiles.close();
    await server.close();
  }
});

test("Ollama discovery preserves actual vision capability and model context limits", async t => {
  const paths = await fixture(t);
  const requests: string[] = [];
  const server = await listen((request, response) => {
    requests.push(request.url ?? "");
    response.setHeader("content-type", "application/json");
    if (request.url === "/api/tags") {
      response.end(JSON.stringify({ models: [{ name: "local-vision" }, { name: "local-text" }] }));
    } else if (request.url === "/api/show") {
      let body = "";
      request.on("data", chunk => { body += String(chunk); });
      request.on("end", () => {
        const value: unknown = JSON.parse(body);
        assert.ok(isRecord(value));
        response.end(JSON.stringify({
          capabilities: value.model === "local-vision" ? ["completion", "tools", "vision"] : ["completion", "tools"],
          model_info: { "model.context_length": 32_768 },
        }));
      });
    } else {
      response.statusCode = 404;
      response.end('{"error":"unexpected request"}');
    }
  });
  const config = configuration(paths);
  config.profiles.test = {
    kind: "open_ai_compatible", auth_method: "api_key", base_url: server.url,
    offline: true, local_runtime: "ollama",
  };
  const profiles = await ModelProfiles.create(config, join(paths.root, "agent"), () => {}, new Redactor());
  try {
    const models = await profiles.models("test");
    assert.ok(Array.isArray(models));
    const vision = models.find(model => isRecord(model) && model.id === "local-vision");
    const text = models.find(model => isRecord(model) && model.id === "local-text");
    assert.ok(isRecord(vision) && vision.vision && vision.context_window === 32_768);
    assert.ok(isRecord(text) && !text.vision && text.context_window === 32_768);
    assert.deepEqual(requests, ["/api/tags", "/api/show", "/api/show"]);
  } finally {
    await profiles.close();
    await server.close();
  }
});

test("real native pi OpenAI-compatible streaming arrives before the model finishes", async t => {
  const paths = await fixture(t);
  const firstDelta = deferred<void>();
  const finish = deferred<void>();
  const requests: string[] = [];
  const server = await listen((request, response) => {
    requests.push(request.url ?? "");
    if (request.url === "/v1/models") {
      response.writeHead(200, { "content-type": "application/json" });
      response.end('{"data":[{"id":"fixture-local","context_window":16384,"max_output_tokens":2048}]}');
      return;
    }
    assert.equal(request.url, "/v1/chat/completions");
    assert.equal(request.headers.authorization, "Bearer local-owner-key");
    response.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
    response.write(`data: ${JSON.stringify({
      id: "completion-fixture", object: "chat.completion.chunk", model: "fixture-local",
      choices: [{ index: 0, delta: { role: "assistant", content: "Live reply" }, finish_reason: null }],
    })}\n\n`);
    void finish.promise.then(() => {
      response.write(`data: ${JSON.stringify({
        id: "completion-fixture", object: "chat.completion.chunk", model: "fixture-local",
        choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
        usage: { prompt_tokens: 12, completion_tokens: 3, total_tokens: 15 },
      })}\n\ndata: [DONE]\n\n`);
      response.end();
    });
  });
  const config = configuration(paths);
  config.profiles.test = {
    kind: "open_ai_compatible", auth_method: "api_key",
    base_url: server.url, api_key: "local-owner-key", offline: true,
  };
  config.routes.chat.model = "fixture-local";
  const notifications: Notification[] = [];
  const worker = new WorkerHost({
    redactor: new Redactor(),
    call: async () => { throw new Error("No domain tool expected"); },
    notify: notification => {
      notifications.push(notification);
      if (notification.method === "event" && notification.params.delta.type === "message_delta") firstDelta.resolve();
    },
  });
  try {
    await worker.request({ id: 1, method: "initialize", params: initializeParams(config) });
    let settled = false;
    const request = worker.request({
      id: 2, method: "chat", params: { text: "Say hello", channel: "text", context: "" },
    }).then(result => { settled = true; return result; });
    const timeout = deferred<string>();
    const timer = setTimeout(() => timeout.resolve("timeout"), 10_000);
    let observed: string;
    try {
      observed = await Promise.race([
        firstDelta.promise.then(() => "delta"),
        request.then(() => "settled"),
        timeout.promise,
      ]);
    } finally {
      clearTimeout(timer);
    }
    assert.equal(observed, "delta");
    assert.equal(settled, false);
    finish.resolve();
    await request;
    assert.deepEqual(requests, ["/v1/models", "/v1/chat/completions"]);
    const deltas = notifications.flatMap(notification => notification.method === "event" ? [notification.params.delta] : []);
    assert.equal(deltas.filter(delta => delta.type === "message_delta").map(delta => delta.text).join(""), "Live reply");
    assert.equal(deltas.at(-1)?.type, "done");
  } finally {
    finish.resolve();
    await worker.close();
    await server.close();
  }
});

test("unsupported OAuth and unsafe generic endpoints fail before execution", async t => {
  const paths = await fixture(t);
  for (const profile of [
    { kind: "rail_relay" as const, auth_method: "oauth" as const, base_url: "https://relay.example.invalid/v1" },
    { kind: "open_ai_compatible" as const, auth_method: "api_key" as const, base_url: "http://remote.example.invalid/v1" },
  ]) {
    const config = configuration(paths);
    config.profiles.test = profile;
    await assert.rejects(ModelProfiles.create(config, join(paths.root, "agent"), () => {}, new Redactor()));
  }
});

test("entry initializes pi's native search cache inside the authority, never ambient HOME", async t => {
  const paths = await fixture(t);
  const ownerBin = join(paths.root, "agent/bin");
  const home = join(paths.root, "ambient-home");
  const ambientBin = join(home, ".pi/agent/bin");
  await mkdir(ownerBin, { recursive: true, mode: 0o700 });
  await mkdir(ambientBin, { recursive: true, mode: 0o700 });
  await writeFile(join(ownerBin, "fd"), "#!/bin/sh\nprintf '%s\\n' authority.txt\n", { mode: 0o700 });
  await writeFile(join(ambientBin, "fd"), "#!/bin/sh\nprintf '%s\\n' wrong-global.txt\n", { mode: 0o700 });
  await writeFile(join(paths.workspace, "authority.txt"), "Owner file.");
  let modelCalls = 0;
  let usedOwnerTool = false;
  const server = await listen((request, response) => {
    if (request.url === "/v1/models") {
      response.writeHead(200, { "content-type": "application/json" });
      response.end('{"data":[{"id":"fixture-local","context_window":16384,"max_output_tokens":2048}]}');
      return;
    }
    const chunks: Buffer[] = [];
    request.on("data", (chunk: Buffer) => chunks.push(chunk));
    request.on("end", () => {
      modelCalls++;
      if (modelCalls > 1) {
        const body = Buffer.concat(chunks).toString();
        usedOwnerTool = body.includes("authority.txt") && !body.includes("wrong-global.txt");
      }
      response.writeHead(200, { "content-type": "text/event-stream" });
      const delta = modelCalls === 1 ? {
        role: "assistant",
        tool_calls: [{ index: 0, id: "native-find", type: "function", function: {
          name: "find", arguments: JSON.stringify({ pattern: "*.txt" }),
        } }],
      } : { role: "assistant", content: "Search complete." };
      response.write(`data: ${JSON.stringify({
        id: `completion-${modelCalls}`, object: "chat.completion.chunk", model: "fixture-local",
        choices: [{ index: 0, delta, finish_reason: null }],
      })}\n\n`);
      response.end(`data: ${JSON.stringify({
        id: `completion-${modelCalls}`, object: "chat.completion.chunk", model: "fixture-local",
        choices: [{ index: 0, delta: {}, finish_reason: modelCalls === 1 ? "tool_calls" : "stop" }],
      })}\n\ndata: [DONE]\n\n`);
    });
  });
  const config = configuration(paths);
  config.profiles.test = {
    kind: "open_ai_compatible", auth_method: "api_key", api_key: "local-test-key",
    base_url: server.url, offline: true,
  };
  config.routes.chat.model = "fixture-local";
  const child = spawn(process.execPath, [fileURLToPath(new URL("./main.js", import.meta.url))], {
    stdio: ["pipe", "pipe", "pipe"],
    env: { ...process.env, HOME: home, PI_CODING_AGENT_DIR: join(home, ".pi/agent") },
  });
  const exited = once(child, "exit");
  let buffer = "";
  let stderr = "";
  let chatError: unknown;
  const processes=new ProcessFixture(params=>child.stdin.write(`${JSON.stringify({method:"process_event",params})}\n`));
  child.stderr.on("data", (chunk: Buffer) => { stderr += chunk.toString(); });
  child.stdout.on("data", (chunk: Buffer) => {
    buffer += chunk.toString();
    let newline = buffer.indexOf("\n");
    while (newline !== -1) {
      const parsed: unknown = JSON.parse(buffer.slice(0, newline));
      const message = object(parsed);
      buffer = buffer.slice(newline + 1);
      if (message.method==="process") {
        void processes.invoke(object(message.params)).then(
          result=>child.stdin.write(`${JSON.stringify({id:message.id,result})}\n`),
          error=>child.stdin.write(`${JSON.stringify({id:message.id,error:{message:String(error)}})}\n`),
        );
      } else if (message.id === 1) {
        chatError = message.error;
        child.stdin.write(`${JSON.stringify({ id: 2, method: "chat", params: {
          text: "Find the text file.", channel: "text", context: "",
        } })}\n`);
      } else if (message.id === 2) {
        chatError ??= message.error;
        child.stdin.write('{"id":3,"method":"shutdown","params":{}}\n');
      }
      newline = buffer.indexOf("\n");
    }
  });
  child.stdin.write(`${JSON.stringify({ id: 1, method: "initialize", params: config })}\n`);
  try {
    const [code] = await exited;
    assert.equal(code, 0, stderr);
    assert.equal(chatError, undefined);
    assert.equal(modelCalls, 2);
    assert.equal(usedOwnerTool, true);
  } finally {
    if (child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL");
      await exited;
    }
    await server.close();
    await processes.close();
  }
});
