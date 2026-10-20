import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { mkdir, readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { configuration, configureMcp, fixture, host, mcpChat, mcpFixture, userText } from "./test-support.ts";
import { WorkerFault } from "./errors.ts";
import { isRecord } from "./protocol.ts";

test("an MCP reload that fails on one live channel restores every channel's previous servers", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  configureMcp(config);
  const starts = join(paths.root, "flaky-starts");
  await mkdir(starts, { mode: 0o700 });
  const contexts = new Map<string, string>();
  const { worker } = await host(t, config, context => {
    contexts.set(userText(context), JSON.stringify(context));
    return { text: "Answered." };
  });
  const second = randomUUID();
  await worker.request({ id: 2, method: "chat", params: { ...mcpChat, text: "Open second", session_key: second } });
  await assert.rejects(worker.request({ id: 3, method: "reload", params: { mcp: { servers: { flaky: {
    command: process.execPath, args: [mcpFixture], env: { BOBE_TEST_START_DIR: starts, BOBE_TEST_START_LIMIT: "2" },
    timeout_ms: 15_000, excluded_tools: [],
  } } } } }), /rolled back/);
  assert.equal((await readdir(starts)).length, 3);
  assert.deepEqual(await worker.request({ id: 4, method: "mcp_status", params: {} }),
    [{ name: "sample", connected: true, status: "connected", error: null }]);
  await worker.request({ id: 5, method: "chat", params: { ...mcpChat, text: "Owner after reload" } });
  await worker.request({ id: 6, method: "chat", params: { ...mcpChat, text: "Second after reload", session_key: second } });
  for (const text of ["Owner after reload", "Second after reload"]) {
    const context = contexts.get(text) ?? "";
    assert.ok(context.includes("mcp__sample__echo"), text);
    assert.equal(context.includes("flaky"), false, text);
  }
  assert.equal((await readdir(starts)).length, 3);
});

test("an MCP reload whose rollback cannot restore the previous servers is a worker fault", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  const server = async (name: string) => {
    const starts = join(paths.root, `${name}-starts`);
    await mkdir(starts, { mode: 0o700 });
    return { starts, config: {
      command: process.execPath, args: [mcpFixture], env: { BOBE_TEST_START_DIR: starts, BOBE_TEST_START_LIMIT: "1" },
      timeout_ms: 15_000, excluded_tools: [],
    } };
  };
  const previous = await server("previous");
  const next = await server("next");
  config.mcp = { servers: { previous: previous.config } };
  const { worker } = await host(t, config, () => ({ text: "unused" }));
  let id = 1;
  await t.waitFor(async () => {
    const status = await worker.request({ id: ++id, method: "mcp_status", params: {} });
    assert.ok(Array.isArray(status) && status.every(entry => isRecord(entry) && entry.connected), "MCP fixture connects");
  }, { interval: 50, timeout: 20_000 });
  await assert.rejects(worker.request({ id: ++id, method: "reload", params: { mcp: { servers: { next: next.config } } } }),
    error => error instanceof WorkerFault && /previous servers could not be restored/.test(error.message));
  assert.equal((await readdir(previous.starts)).length, 2);
  assert.equal((await readdir(next.starts)).length, 2);
});

test("an MCP server that failed after connecting is restarted by the next turn", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  configureMcp(config);
  config.mcp.servers.sample!.env.BOBE_TEST_EXIT_ON_CALL = "3";
  let calls = 0;
  const { worker } = await host(t, config, context => {
    if (userText(context) === "After crash") return { text: "Restarted." };
    return ++calls === 1
      ? { tools: [{ id: "mcp-crash", name: "mcp__sample__echo", arguments: { text: "crash" } }] }
      : { text: "The tool failed." };
  });
  await worker.request({ id: 2, method: "chat", params: mcpChat });
  const pid = Number(await readFile(join(paths.workspace, "mcp.pid"), "utf8"));
  const status = await worker.request({ id: 3, method: "mcp_status", params: {} });
  assert.ok(Array.isArray(status) && isRecord(status[0]) && status[0].status === "failed", JSON.stringify(status));
  await worker.request({ id: 4, method: "chat", params: { ...mcpChat, text: "After crash" } });
  assert.notEqual(Number(await readFile(join(paths.workspace, "mcp.pid"), "utf8")), pid);
});
