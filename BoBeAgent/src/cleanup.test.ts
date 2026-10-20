import assert from "node:assert/strict";
import test from "node:test";
import { access, readFile } from "node:fs/promises";
import { join } from "node:path";
import { configuration, configureMcp, fixture, host, mcpChat, userText } from "./test-support.ts";
import { WorkerFault } from "./errors.ts";
import { isRecord } from "./protocol.ts";

test("a run whose cleanup cannot stop its MCP server is a worker fault and leaves no subscriber behind", async t => {
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
  const rejected = assert.rejects(running,
    error => error instanceof WorkerFault && /Run cleanup failed: .*Injected process stop failure/.test(error.message));
  await t.waitFor(() => access(waiting), { interval: 10, timeout: 10_000 });
  const pid = Number(await readFile(join(paths.workspace, "mcp.pid"), "utf8"));
  processes.failingStops = 1;
  assert.deepEqual(await worker.request({ id: 3, method: "abort", params: { request_id: 2 } }), {});
  await rejected;
  await worker.request({ id: 4, method: "chat", params: { ...mcpChat, text: "After failed cleanup" } });
  const deltas = notifications.flatMap(notification =>
    notification.method === "event" && notification.params.request_id === 4 ? [notification.params.delta] : []);
  assert.equal(deltas.filter(delta => delta.type === "message_delta").map(delta => delta.text).join(""), "Recovered.");
  assert.equal(deltas.filter(delta => delta.type === "done").length, 1);
  assert.throws(() => process.kill(pid, 0), error => isRecord(error) && error.code === "ESRCH");
});

test("a parent turn that succeeds after a subagent could not stop its MCP server is a worker fault", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  configureMcp(config);
  const { worker, notifications, processes } = await host(t, config, context => {
    if (userText(context) === "child task") {
      processes.failingStops = 100;
      return { text: "Child finished." };
    }
    return context.messages.some(message => message.role === "toolResult" && message.toolName === "subagent")
      ? { text: "Parent finished." }
      : { tools: [{ id: "delegate-1", name: "subagent", arguments: { mode: "parallel", tasks: [{ task: "child task" }] } }] };
  });
  await assert.rejects(worker.request({ id: 2, method: "chat", params: { ...mcpChat, text: "Delegate" } }),
    error => error instanceof WorkerFault && /Session cleanup failed: .*Injected process stop failure/.test(error.message));
  processes.failingStops = 0;
  const deltas = notifications.flatMap(notification =>
    notification.method === "event" && notification.params.request_id === 2 ? [notification.params.delta] : []);
  assert.ok(deltas.some(delta => delta.type === "tool_complete" && delta.name === "subagent" && !delta.success));
  assert.equal(deltas.filter(delta => delta.type === "message_delta").map(delta => delta.text).join(""), "Parent finished.");
  assert.equal(deltas.some(delta => delta.type === "done"), false);
});
