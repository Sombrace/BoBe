import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import { configuration, deferred, fixture, host, userText } from "./test-support.js";
import { abortableDelay } from "./errors.js";
import { isRecord, type Initialize, type JsonObject } from "./protocol.js";
import type { WorkerHost } from "./worker.js";

function configure(config: Initialize): void {
  config.mcp = { servers: { sample: {
    command: process.execPath,
    args: [fileURLToPath(new URL("./mcp-fixture.js", import.meta.url))],
    env: { BOBE_TEST_PID_FILE: join(config.workspace, "mcp.pid") },
    timeout_ms: 3_000,
    excluded_tools: ["hidden"],
  } } };
}

async function connected(worker: WorkerHost): Promise<void> {
  for (let count = 0; count < 300; count++) {
    const status = await worker.request({ id: 100 + count, method: "mcp_status", params: {} });
    if (Array.isArray(status) && status.every(server => isRecord(server) && server.connected)) return;
    await abortableDelay(10);
  }
  throw new Error("MCP fixture did not connect");
}

const chat: JsonObject = { text: "Use MCP", channel: "text", context: "" };

test("native pi MCP connects, honors exclusions, executes, reloads, and closes its process", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  configure(config);
  config.mcp.servers.sample!.env.BOBE_TEST_PAGINATION = "1";
  let count = 0;
  const { worker } = await host(t, config, context => {
    assert.equal(JSON.stringify(context).includes("mcp__sample__hidden"), false);
    return ++count === 1
      ? { tools: [{ id: "mcp-1", name: "mcp__sample__echo", arguments: { text: "echo value" } }] }
      : { text: "MCP completed." };
  });
  await connected(worker);
  const tools = await worker.request({ id: 500, method: "mcp_tools", params: { server_name: "sample" } });
  assert.deepEqual(tools, [
    { name: "echo", description: "Controlled echo tool" },
    { name: "wait", description: "Controlled wait tool" },
  ]);
  const pid = Number(await readFile(join(paths.workspace, "mcp.pid"), "utf8"));
  await worker.request({ id: 501, method: "chat", params: chat });
  assert.equal(count, 2);
  await worker.request({ id: 502, method: "reload", params: { mcp: { servers: {} } } });
  assert.deepEqual(await worker.request({ id: 503, method: "mcp_status", params: {} }), []);
  assert.throws(() => process.kill(pid, 0), error => isRecord(error) && error.code === "ESRCH");
});

test("native MCP in-flight tools settle on abort and owned servers stop at shutdown", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  configure(config);
  const toolRequested = deferred<void>();
  const { worker, notifications } = await host(t, config, context => {
    if (userText(context) === "After abort") return { text: "Recovered." };
    toolRequested.resolve();
    return { tools: [{ id: "mcp-wait", name: "mcp__sample__wait", arguments: { text: "never finish" } }] };
  });
  await connected(worker);
  const pid = Number(await readFile(join(paths.workspace, "mcp.pid"), "utf8"));
  const pointer = await readFile(join(paths.root, "agent/session.json"), "utf8");
  const running = worker.request({ id: 500, method: "chat", params: chat });
  const rejected = assert.rejects(running);
  await toolRequested.promise;
  for (let attempt = 0; attempt < 100; attempt++) {
    if (notifications.some(notification => notification.method === "event" &&
      notification.params.delta.type === "tool_start")) break;
    await abortableDelay(5);
  }
  await worker.request({ id: 501, method: "abort", params: { request_id: 500 } });
  await rejected;
  assert.throws(() => process.kill(pid, 0), error => isRecord(error) && error.code === "ESRCH");
  await worker.request({ id: 502, method: "chat", params: { ...chat, text: "After abort" } });
  assert.equal(await readFile(join(paths.root, "agent/session.json"), "utf8"), pointer);
  const restartedPid = Number(await readFile(join(paths.workspace, "mcp.pid"), "utf8"));
  assert.notEqual(restartedPid, pid);
  await worker.close();
  assert.throws(() => process.kill(restartedPid, 0), error => isRecord(error) && error.code === "ESRCH");
});

test("MCP reload reports a real connection failure", async t => {
  const paths = await fixture(t);
  const { worker } = await host(t, configuration(paths), () => ({ text: "unused" }));
  await assert.rejects(worker.request({
    id: 2, method: "reload", params: { mcp: { servers: { broken: {
      command: join(paths.root, "missing-program"), args: [], env: {}, timeout_ms: 1_000, excluded_tools: [],
    } } } },
  }), /MCP/);
  const status = await worker.request({ id: 3, method: "mcp_status", params: {} });
  assert.ok(Array.isArray(status) && isRecord(status[0]) && status[0].connected === false);
});
