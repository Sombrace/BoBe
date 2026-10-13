import assert from "node:assert/strict";
import test from "node:test";
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { once } from "node:events";
import { join } from "node:path";
import { readFile } from "node:fs/promises";
import { ModelProfiles } from "./models.js";
import { Redactor } from "./errors.js";
import { configuration, fixture } from "./test-support.js";
import { WorkerHost, initializeParams } from "./worker.js";
import { isRecord, type Json, type Notification } from "./protocol.js";

const responsesCatalog = [
  { id: "responses-model", api: "responses", context_window: 128_000, max_input_tokens: 96_000, max_prompt_tokens: 64_000 },
  { id: "chat-model", api: "chat", max_input_tokens: 0, max_prompt_tokens: 32_000 },
  { id: "shared-model", api: "responses", max_input_tokens: 48_000, max_prompt_tokens: 0 },
];
const messagesCatalog = [
  { id: "messages-model", api: "messages", max_input_tokens: 120_000, max_prompt_tokens: 80_000, vision: true },
  { id: "shared-model", api: "messages", max_input_tokens: 48_000 },
];

async function relayServer(
  handle: (request: IncomingMessage, response: ServerResponse) => void,
): Promise<{ url: string; close(): Promise<void> }> {
  const server = createServer(handle);
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

function catalog(request: IncomingMessage, response: ServerResponse, emptyResponses = false): boolean {
  if (request.url !== "/v1/models") return false;
  const messages = request.headers["x-api-key"] !== undefined || request.headers["anthropic-version"] !== undefined;
  response.writeHead(200, { "content-type": "application/json" });
  response.end(JSON.stringify({ data: messages ? messagesCatalog : emptyResponses ? [] : responsesCatalog }));
  return true;
}

function sse(response: ServerResponse, events: { type: string; [key: string]: Json }[]): void {
  response.writeHead(200, { "content-type": "text/event-stream" });
  for (const event of events) response.write(`event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`);
  response.end();
}

function messagesResponse(response: ServerResponse): void {
  sse(response, [
    { type: "message_start", message: {
      id: "message-fixture", type: "message", role: "assistant", content: [], model: "messages-model",
      stop_reason: null, stop_sequence: null, usage: { input_tokens: 12, output_tokens: 0 },
    } },
    { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } },
    { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Messages works." } },
    { type: "content_block_stop", index: 0 },
    { type: "message_delta", delta: { stop_reason: "end_turn", stop_sequence: null }, usage: { output_tokens: 3 } },
    { type: "message_stop" },
  ]);
}

function responsesResponse(response: ServerResponse): void {
  const item = {
    id: "output-fixture", type: "message", role: "assistant", status: "completed",
    content: [{ type: "output_text", text: "Responses works.", annotations: [] }],
  };
  sse(response, [
    { type: "response.created", response: { id: "response-fixture", status: "in_progress", output: [], model: "responses-model" } },
    { type: "response.output_item.added", output_index: 0, item: { ...item, status: "in_progress", content: [] } },
    { type: "response.content_part.added", output_index: 0, content_index: 0, item_id: item.id, part: { type: "output_text", text: "", annotations: [] } },
    { type: "response.output_text.delta", output_index: 0, content_index: 0, item_id: item.id, delta: "Responses works." },
    { type: "response.output_text.done", output_index: 0, content_index: 0, item_id: item.id, text: "Responses works." },
    { type: "response.output_item.done", output_index: 0, item },
    { type: "response.completed", response: {
      id: "response-fixture", status: "completed", model: "responses-model", output: [item],
      usage: { input_tokens: 12, output_tokens: 3, total_tokens: 15 },
    } },
  ]);
}

function chatResponse(response: ServerResponse): void {
  response.writeHead(200, { "content-type": "text/event-stream" });
  response.write(`data: ${JSON.stringify({
    id: "chat-fixture", object: "chat.completion.chunk", model: "chat-model",
    choices: [{ index: 0, delta: { role: "assistant", content: "Chat works." }, finish_reason: null }],
  })}\n\n`);
  response.end(`data: ${JSON.stringify({
    id: "chat-fixture", object: "chat.completion.chunk", model: "chat-model",
    choices: [{ index: 0, delta: {}, finish_reason: "stop" }],
    usage: { prompt_tokens: 12, completion_tokens: 3, total_tokens: 15 },
  })}\n\ndata: [DONE]\n\n`);
}

test("Relay catalog unions both authenticated views and applies the lowest positive input bound", async t => {
  const paths = await fixture(t);
  const views: string[] = [];
  const server = await relayServer((request, response) => {
    if (request.headers["x-api-key"]) {
      assert.equal(request.headers["x-api-key"], "relay-fixture-token");
      assert.equal(request.headers["anthropic-version"], "2023-06-01");
      assert.equal(request.headers.authorization, undefined);
      views.push("messages");
    } else {
      assert.equal(request.headers.authorization, "Bearer relay-fixture-token");
      assert.equal(request.headers["anthropic-version"], undefined);
      views.push("responses");
    }
    assert.equal(catalog(request, response), true);
  });
  const config = configuration(paths);
  config.profiles.test = { kind: "rail_relay", auth_method: "api_key", api_key: "relay-fixture-token", base_url: server.url };
  const profiles = await ModelProfiles.create(config, join(paths.root, "agent"), () => {}, new Redactor());
  try {
    const models = await profiles.models("test");
    assert.ok(Array.isArray(models));
    assert.deepEqual(models.map(model => isRecord(model) ? [model.id, model.context_window] : null), [
      ["responses-model", 64_000], ["chat-model", 32_000], ["shared-model", 48_000], ["messages-model", 80_000],
    ]);
    assert.deepEqual(views.sort(), ["messages", "responses"]);
    const shared = await profiles.select({ profile: "test", model: "shared-model", reasoning_effort: null });
    assert.equal(shared.api, "openai-responses");
    const messages = await profiles.select({ profile: "test", model: "messages-model", reasoning_effort: null });
    assert.equal(messages.api, "anthropic-messages");
    assert.equal(messages.baseUrl, server.url.replace(/\/v1$/, ""));
    assert.ok(messages.input.includes("image"));
  } finally {
    await profiles.close();
    await server.close();
  }
});

test("Relay capability metadata preserves vision, supported reasoning levels and output limits", async t => {
  const paths = await fixture(t);
  const server = await relayServer((_request, response) => {
    response.setHeader("content-type", "application/json");
    response.end(JSON.stringify({ data: [{
      id: "capability-model", api: "responses", context_window: 1_050_000,
      max_prompt_tokens: 922_000, max_output_tokens: 128_000,
      capabilities: { image_input: true, reasoning: true, effort: ["none", "low", "medium", "high", "xhigh", "max"] },
    }] }));
  });
  const config = configuration(paths);
  config.profiles.test = { kind: "rail_relay", auth_method: "api_key", api_key: "relay-fixture-token", base_url: server.url };
  const profiles = await ModelProfiles.create(config, join(paths.root, "agent"), () => {}, new Redactor());
  try {
    const models = await profiles.models("test");
    assert.ok(Array.isArray(models));
    const model = models.find(value => isRecord(value) && value.id === "capability-model");
    assert.ok(isRecord(model));
    assert.equal(model.vision, true);
    assert.deepEqual(model.supported_reasoning_efforts, ["off", "low", "medium", "high", "xhigh", "max"]);
    const selected = await profiles.select({ profile: "test", model: "capability-model", reasoning_effort: "low" });
    assert.equal(selected.contextWindow, 922_000);
    assert.equal(selected.maxTokens, 128_000);
    assert.equal(selected.reasoning, true);
  } finally {
    await profiles.close();
    await server.close();
  }
});

test("concurrent catalog refresh and model selection keep a coherent provider catalog", async t => {
  const paths = await fixture(t);
  const server = await relayServer((request, response) => {
    setTimeout(() => { catalog(request, response); }, 20);
  });
  const config = configuration(paths);
  config.profiles.test = { kind: "rail_relay", auth_method: "api_key", api_key: "relay-fixture-token", base_url: server.url };
  const profiles = await ModelProfiles.create(config, join(paths.root, "agent"), () => {}, new Redactor());
  try {
    const [first, selected, second] = await Promise.all([
      profiles.models("test"),
      profiles.select({ profile: "test", model: "responses-model", reasoning_effort: null }),
      profiles.models("test"),
    ]);
    assert.ok(Array.isArray(first) && first.length === 4);
    assert.equal(selected.id, "responses-model");
    assert.deepEqual(second, first);
  } finally {
    await profiles.close();
    await server.close();
  }
});

test("changing turn context preserves the exact preceding Responses request prefix", async t => {
  const paths = await fixture(t);
  const requests: unknown[] = [];
  const server = await relayServer((request, response) => {
    if (catalog(request, response)) return;
    let body = "";
    request.on("data", chunk => { body += String(chunk); });
    request.on("end", () => {
      requests.push(JSON.parse(body));
      responsesResponse(response);
    });
  });
  const config = configuration(paths);
  config.profiles.test = { kind: "rail_relay", auth_method: "api_key", api_key: "relay-fixture-token", base_url: server.url };
  config.routes.chat.model = "responses-model";
  const worker = new WorkerHost({
    redactor: new Redactor(), call: async () => { throw new Error("No tool expected"); }, notify: () => {},
  });
  try {
    await worker.request({ id: 1, method: "initialize", params: initializeParams(config) });
    for (const [index, context] of ["CLOCK_ONE", "CLOCK_TWO"].entries()) {
      await worker.request({ id: index + 2, method: "chat", params: {
        text: `Turn ${index + 1}`, channel: "text", context,
      } });
    }
    const first = requests[0];
    const second = requests[1];
    assert.ok(isRecord(first) && isRecord(second));
    assert.ok(Array.isArray(first.input) && Array.isArray(second.input));
    assert.ok(JSON.stringify(second.input.slice(0, first.input.length)) === JSON.stringify(first.input),
      "Previously sent request input must stay byte-for-byte stable when the turn clock changes");
    assert.equal(second.prompt_cache_key, first.prompt_cache_key);
  } finally {
    await worker.close();
    await server.close();
  }
});

test("Relay does not treat an empty Responses view as an empty universal catalog", async t => {
  const paths = await fixture(t);
  const server = await relayServer((request, response) => { catalog(request, response, true); });
  const config = configuration(paths);
  config.profiles.test = { kind: "rail_relay", auth_method: "api_key", api_key: "relay-fixture-token", base_url: server.url };
  const profiles = await ModelProfiles.create(config, join(paths.root, "agent"), () => {}, new Redactor());
  try {
    const models = await profiles.models("test");
    assert.ok(Array.isArray(models) && models.length === 2);
    assert.ok(models.some(model => isRecord(model) && model.id === "messages-model"));
  } finally {
    await profiles.close();
    await server.close();
  }
});

test("Relay catalog view failure is explicit rather than a silently incomplete model list", async t => {
  const paths = await fixture(t);
  const server = await relayServer((request, response) => {
    if (request.headers["x-api-key"]) {
      response.writeHead(503);
      response.end();
    } else catalog(request, response);
  });
  const config = configuration(paths);
  config.profiles.test = { kind: "rail_relay", auth_method: "api_key", api_key: "relay-fixture-token", base_url: server.url };
  const profiles = await ModelProfiles.create(config, join(paths.root, "agent"), () => {}, new Redactor());
  try {
    await assert.rejects(profiles.models("test"), /HTTP 503/);
  } finally {
    await profiles.close();
    await server.close();
  }
});

test("Relay api metadata routes real pi sessions to Messages, Responses, and Chat without changing identity", async t => {
  const paths = await fixture(t);
  const executionPaths: string[] = [];
  const server = await relayServer((request, response) => {
    if (catalog(request, response)) return;
    const path = new URL(request.url ?? "/", "http://127.0.0.1").pathname;
    executionPaths.push(path);
    if (path === "/v1/messages") {
      assert.equal(request.headers["x-api-key"], "relay-fixture-token");
      messagesResponse(response);
    } else if (path === "/v1/responses") {
      assert.equal(request.headers.authorization, "Bearer relay-fixture-token");
      responsesResponse(response);
    } else if (path === "/v1/chat/completions") {
      assert.equal(request.headers.authorization, "Bearer relay-fixture-token");
      chatResponse(response);
    } else {
      response.writeHead(404);
      response.end();
    }
  });
  let pointer: string | undefined;
  try {
    for (const [model, expected] of [
      ["messages-model", "Messages works."],
      ["responses-model", "Responses works."],
      ["chat-model", "Chat works."],
    ]) {
      assert.ok(model && expected);
      const config = configuration(paths);
      config.profiles.test = {
        kind: "rail_relay", auth_method: "api_key", api_key: "relay-fixture-token",
        base_url: model === "responses-model" ? server.url.replace(/\/v1$/, "") : server.url,
      };
      config.routes.chat.model = model;
      const notifications: Notification[] = [];
      const worker = new WorkerHost({
        redactor: new Redactor(), notify: event => notifications.push(event),
        call: async () => { throw new Error("Unexpected domain tool"); },
      });
      try {
        await worker.request({ id: 1, method: "initialize", params: initializeParams(config) });
        await worker.request({ id: 2, method: "chat", params: {
          text: "Return a short response.", channel: "text", context: "",
        } });
        const text = notifications.flatMap(notification => notification.method === "event" &&
          notification.params.delta.type === "message_delta" ? [notification.params.delta.text] : []).join("");
        assert.equal(text, expected);
        const currentPointer = await readFile(join(paths.root, "agent/session.json"), "utf8");
        pointer ??= currentPointer;
        assert.equal(currentPointer, pointer);
      } finally {
        await worker.close();
      }
    }
    assert.deepEqual(executionPaths, ["/v1/messages", "/v1/responses", "/v1/chat/completions"]);
  } finally {
    await server.close();
  }
});
