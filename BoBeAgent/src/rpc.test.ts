import assert from "node:assert/strict";
import test from "node:test";
import { PassThrough, Writable } from "node:stream";
import { JsonlRpc } from "./rpc.js";
import { MAX_LINE_BYTES, object, type JsonObject } from "./protocol.js";
import { deferred } from "./test-support.js";

function wire() {
  const input = new PassThrough();
  const output = new PassThrough();
  const messages: JsonObject[] = [];
  const waiters: { predicate: (message: JsonObject) => boolean; resolve(message: JsonObject): void }[] = [];
  output.on("data", (chunk: Buffer) => {
    for (const line of chunk.toString("utf8").trimEnd().split("\n")) {
      const parsed: unknown = JSON.parse(line);
      const message = object(parsed);
      messages.push(message);
      for (const waiter of [...waiters]) {
        if (waiter.predicate(message)) {
          waiters.splice(waiters.indexOf(waiter), 1);
          waiter.resolve(message);
        }
      }
    }
  });
  const next = (predicate: (message: JsonObject) => boolean): Promise<JsonObject> => {
    const existing = messages.find(predicate);
    if (existing) return Promise.resolve(existing);
    return new Promise(resolve => waiters.push({ predicate, resolve }));
  };
  return { input, output, messages, next, rpc: new JsonlRpc(input, output) };
}

test("a stalled RPC consumer has a bounded output queue and fails explicitly", async () => {
  const input = new PassThrough();
  const blocked = deferred<(error?: Error | null) => void>();
  const output = new Writable({ write(_chunk, _encoding, callback) { blocked.resolve(callback); } });
  const rpc = new JsonlRpc(input, output);
  const running = rpc.start({ request: async () => ({}), close: async () => {} });
  const failed = assert.rejects(running, /output queue capacity/);
  assert.throws(() => {
    for (let index = 0; index < 1025; index++) rpc.notify({ method: "event", params: {
      request_id: 1, delta: { type: "tool_start", id: String(index), name: "read" },
    } });
  }, /output queue capacity/);
  (await blocked.promise)(new Error("stalled test output released"));
  await failed;
  assert.equal(input.destroyed, true);
});

test("JSONL accepts concurrent requests and correlates out-of-order Rust tool callbacks", async () => {
  const { input, messages, next, rpc } = wire();
  let closes = 0;
  const running = rpc.start({
    request: request => request.method === "work"
      ? rpc.call("domain_tool", { name: "bobe_test", arguments: { request: request.id } })
      : Promise.resolve({ pong: request.id }),
    close: async () => { closes++; },
  });
  input.write('{"id":1,"method":"work","params":{}}\n{"id":2,"method":"work","params":{}}\n');
  const first = await next(message => message.id === -1);
  const second = await next(message => message.id === -2);
  assert.equal(first.method, "domain_tool");
  assert.equal(second.method, "domain_tool");
  input.write('{"id":3,"method":"ping","params":{}}\n');
  await next(message => message.id === 3);
  assert.equal(messages.some(message => message.id === 1), false);
  input.write('{"id":-2,"result":{"result":"second"}}\n');
  assert.deepEqual((await next(message => message.id === 2)).result, { result: "second" });
  input.write('{"id":-1,"result":{"result":"first"}}\n');
  assert.deepEqual((await next(message => message.id === 1)).result, { result: "first" });
  input.end();
  await running;
  assert.equal(closes, 1);
});

test("callback errors are explicit and do not block other request replies", async () => {
  const { input, next, rpc } = wire();
  const running = rpc.start({
    request: () => rpc.call("domain_tool", {}),
    close: async () => {},
  });
  input.write('{"id":8,"method":"work","params":{}}\n');
  await next(message => message.id === -1);
  input.write('{"id":-1,"error":{"message":"owner tool failed"}}\n');
  const reply = await next(message => message.id === 8);
  assert.deepEqual(reply.error, { message: "owner tool failed" });
  input.end();
  await running;
});

test("split UTF-8 and several frames per chunk preserve exact text", async () => {
  const { input, next, rpc } = wire();
  const running = rpc.start({ request: async request => request.params, close: async () => {} });
  const encoded = Buffer.from('{"id":1,"method":"echo","params":{"text":"BoBe 🙂"}}\n');
  const split = encoded.indexOf(Buffer.from("🙂")) + 2;
  input.write(encoded.subarray(0, split));
  input.write(encoded.subarray(split));
  assert.deepEqual((await next(message => message.id === 1)).result, { text: "BoBe 🙂" });
  input.end();
  await running;
});

test("unknown responses, duplicate request IDs, malformed frames, and oversized lines fail", async t => {
  for (const [name, payload] of [
    ["unknown response", '{"id":-9,"result":{}}\n'],
    ["duplicate", '{"id":1,"method":"echo","params":{}}\n{"id":1,"method":"echo","params":{}}\n'],
    ["malformed", '{"id":1,"method":"echo","params":[]}\n'],
    ["invalid JSON", "not-json\n"],
    ["oversized", " ".repeat(MAX_LINE_BYTES + 1)],
  ]) {
    await t.test(name!, async () => {
      const { input, rpc } = wire();
      const running = rpc.start({ request: async () => ({}), close: async () => {} });
      input.write(payload);
      await assert.rejects(running);
    });
  }
});

test("EOF fails a partial frame and settles outstanding callbacks before shutdown", async () => {
  const { input, next, rpc } = wire();
  const callbackSettled = deferred<void>();
  const running = rpc.start({
    request: async () => {
      try {
        return await rpc.call("domain_tool", {});
      } finally {
        callbackSettled.resolve();
      }
    },
    close: async () => { await callbackSettled.promise; },
  });
  input.write('{"id":1,"method":"work","params":{}}\n');
  await next(message => message.id === -1);
  input.end('{"id":2');
  await assert.rejects(running, /Incomplete/);
  await callbackSettled.promise;
});

test("callback cancellation does not report settled before its owner response", async () => {
  const { input, next, rpc } = wire();
  const controller = new AbortController();
  const running = rpc.start({
    request: () => rpc.call("domain_tool", {}, controller.signal),
    close: async () => {},
  });
  input.write('{"id":1,"method":"work","params":{}}\n');
  await next(message => message.id === -1);
  controller.abort();
  input.write('{"id":-1,"result":{}}\n');
  const response = await next(message => message.id === 1);
  assert.deepEqual(response.error, { message: "Operation canceled" });
  input.end();
  await running;
});

test("shutdown failures close the input and reject the transport instead of hanging", async () => {
  const { input, rpc } = wire();
  const running = rpc.start({
    request: async () => ({}),
    close: async () => { throw new Error("owned resource cleanup failed"); },
  });
  input.write('{"id":1,"method":"shutdown","params":{}}\n');
  await assert.rejects(running, /cleanup failed/);
  assert.equal(input.destroyed, true);
});

test("an oversized callback fails its own request and does not leave an orphan waiter", async () => {
  const { input, next, rpc } = wire();
  const running = rpc.start({
    request: () => rpc.call("domain_tool", { payload: "x".repeat(MAX_LINE_BYTES) }),
    close: async () => {},
  });
  input.write('{"id":1,"method":"work","params":{}}\n');
  assert.deepEqual((await next(message => message.id === 1)).error, { message: "RPC output exceeds 16 MiB" });
  input.end();
  await running;
});
