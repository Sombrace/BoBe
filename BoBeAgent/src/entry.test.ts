import assert from "node:assert/strict";
import test from "node:test";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { configuration, fixture } from "./test-support.js";
import { object, type JsonObject } from "./protocol.js";

test("private worker entry uses protocol-only stdout, initializes without login, and exits on shutdown", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  config.profiles.test!.api_key = null;
  config.routes.chat.model = null;
  const child = spawn(process.execPath, [fileURLToPath(new URL("./main.js", import.meta.url))], {
    stdio: ["pipe", "pipe", "pipe"],
    env: { ...process.env, OPENAI_API_KEY: "ambient-unowned-test-key" },
  });
  const exited = once(child, "exit");
  const messages: JsonObject[] = [];
  let stderr = "";
  let buffer = "";
  child.stderr.on("data", (chunk: Buffer) => { stderr += chunk.toString(); });
  child.stdout.on("data", (chunk: Buffer) => {
    buffer += chunk.toString();
    let newline = buffer.indexOf("\n");
    while (newline !== -1) {
      const parsed: unknown = JSON.parse(buffer.slice(0, newline));
      const message = object(parsed);
      messages.push(message);
      buffer = buffer.slice(newline + 1);
      if (message.id === 1) {
        child.stdin.write('{"id":2,"method":"auth_status","params":{"profile":"test"}}\n');
      } else if (message.id === 2) {
        child.stdin.write('{"id":3,"method":"shutdown","params":{}}\n');
      }
      newline = buffer.indexOf("\n");
    }
  });
  child.stdin.write(`${JSON.stringify({ id: 1, method: "initialize", params: config })}\n`);
  try {
    const [code, signal] = await exited;
    assert.equal(signal, null);
    assert.equal(code, 0, stderr);
    assert.equal(stderr, "");
    assert.deepEqual(messages.map(message => message.id), [1, 2, 3]);
    assert.deepEqual(messages[0]?.result, { protocol_version: 1, harness: "pi", version: "1.2.0" });
    assert.equal(object(messages[1]?.result).is_authenticated, false);
    assert.ok(await readFile(join(paths.root, "agent/session.json"), "utf8"));
  } finally {
    if (child.exitCode === null && child.signalCode === null) {
      child.kill("SIGKILL");
      await exited;
    }
  }
});

test("malformed entry input fails without echoing credential or prompt bytes to diagnostics", async t => {
  const paths = await fixture(t);
  const child = spawn(process.execPath, [fileURLToPath(new URL("./main.js", import.meta.url))], {
    cwd: paths.workspace, stdio: ["pipe", "pipe", "pipe"],
  });
  const exited = once(child, "exit");
  let stderr = "";
  child.stderr.on("data", (chunk: Buffer) => { stderr += chunk.toString(); });
  child.stdin.end("private-secret-invalid-json\n");
  const [code] = await exited;
  assert.equal(code, 1);
  assert.equal(stderr.includes("private-secret"), false);
  assert.ok(stderr.includes("Malformed RPC JSON"));
});
