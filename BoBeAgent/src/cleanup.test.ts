import assert from "node:assert/strict";
import test from "node:test";
import { access, readFile } from "node:fs/promises";
import { join } from "node:path";
import { configuration, configureMcp, fixture, host, mcpChat, userText } from "./test-support.js";
import { abortableDelay } from "./errors.js";
import { isRecord } from "./protocol.js";

async function created(path: string): Promise<void> {
  for (let attempt = 0; attempt < 1_000; attempt++) {
    try {
      await access(path);
      return;
    } catch (error) {
      if (!isRecord(error) || error.code !== "ENOENT") throw error;
      await abortableDelay(10);
    }
  }
  throw new Error(`${path} was not created`);
}

test("a failed MCP stop fails the abort, still settles the run, and the next turn starts clean", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  configureMcp(config);
  const waiting = join(paths.workspace, "mcp-wait-called");
  config.mcp.servers.sample!.env.BOBE_TEST_CALL_FILE = waiting;
  const { worker, notifications, processes } = await host(t, config, context => {
    if (userText(context) === "After failed cleanup") return { text: "Recovered." };
    return { tools: [{ id: "mcp-wait", name: "mcp__sample__wait", arguments: { text: "never finish" } }] };
  });
  const running = worker.request({ id: 2, method: "chat", params: mcpChat });
  const rejected = assert.rejects(running, /cleanup/);
  await created(waiting);
  const pid = Number(await readFile(join(paths.workspace, "mcp.pid"), "utf8"));
  processes.failingStops = 1;
  await assert.rejects(worker.request({ id: 3, method: "abort", params: { request_id: 2 } }),
    /did not release its resources: .*Injected process stop failure/);
  await rejected;
  await worker.request({ id: 4, method: "chat", params: { ...mcpChat, text: "After failed cleanup" } });
  const deltas = notifications.flatMap(notification =>
    notification.method === "event" && notification.params.request_id === 4 ? [notification.params.delta] : []);
  assert.equal(deltas.filter(delta => delta.type === "message_delta").map(delta => delta.text).join(""), "Recovered.");
  assert.equal(deltas.filter(delta => delta.type === "done").length, 1);
  assert.throws(() => process.kill(pid, 0), error => isRecord(error) && error.code === "ESRCH");
});

test("a run whose deadline left an MCP process unreleased fails Rust's follow-up abort once", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  configureMcp(config);
  config.workload_budgets.chat.wall_clock = { secs: 6, nanos: 0 };
  const { worker, processes } = await host(t, config, () => ({
    tools: [{ id: "mcp-wait", name: "mcp__sample__wait", arguments: { text: "never finish" } }],
  }));
  let id = 1;
  for (let attempt = 0; ; attempt++) {
    const status = await worker.request({ id: ++id, method: "mcp_status", params: {} });
    if (Array.isArray(status) && status.every(server => isRecord(server) && server.connected)) break;
    if (attempt === 400) throw new Error("MCP fixture did not connect");
    await abortableDelay(50);
  }
  processes.failingStops = 1;
  const chat = ++id;
  await assert.rejects(worker.request({ id: chat, method: "chat", params: mcpChat }),
    /cleanup failed: .*Injected process stop failure/);
  await assert.rejects(worker.request({ id: ++id, method: "abort", params: { request_id: chat } }),
    /did not release its resources: .*Injected process stop failure/);
  assert.deepEqual(await worker.request({ id: ++id, method: "abort", params: { request_id: chat } }), {});
});
