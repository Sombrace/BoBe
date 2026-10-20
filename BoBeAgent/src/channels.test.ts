import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { configuration, fixture, host, userText } from "./test-support.ts";
import { isRecord } from "./protocol.ts";

test("retiring live channel resources preserves each durable conversation", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  config.execution.live_channels = 1;
  const { worker } = await host(t, config, () => ({ text: "Retained context." }));
  const first = randomUUID();
  const second = randomUUID();
  const ask = (id: number, key: string) => worker.request({ id, method: "chat", params: {
    text: "Keep this channel", session_key: key, channel: "text", context: "",
  } });
  await ask(2, first);
  const original = await worker.request({ id: 3, method: "context_status", params: { session_key: first } });
  await ask(4, second);
  await ask(5, first);
  const restored = await worker.request({ id: 6, method: "context_status", params: { session_key: first } });
  assert.ok(isRecord(original) && isRecord(restored));
  assert.equal(original.session_id, restored.session_id);
  assert.equal(restored.user_messages, 2);
  assert.equal(restored.live_channels, 1);
});

test("idle live channels retire on the owner's schedule and later turns restore them", async t => {
  t.mock.timers.enable({ apis: ["setInterval", "Date"], now: Date.now() });
  const paths = await fixture(t);
  const config = configuration(paths);
  config.execution.idle_channel_seconds = 30;
  const { worker } = await host(t, config, () => ({ text: "Retained context." }));
  const idle = randomUUID();
  await worker.request({ id: 2, method: "chat", params: {
    text: "Keep this channel", session_key: idle, channel: "text", context: "",
  } });
  const original = await worker.request({ id: 3, method: "context_status", params: { session_key: idle } });
  t.mock.timers.tick(20_000);
  const active = await worker.request({ id: 4, method: "context_status", params: { session_key: randomUUID() } });
  t.mock.timers.tick(20_000);
  const retired = await worker.request({ id: 5, method: "context_status", params: { session_key: randomUUID() } });
  const restored = await worker.request({ id: 6, method: "context_status", params: { session_key: idle } });
  assert.ok(isRecord(original) && isRecord(active) && isRecord(retired) && isRecord(restored));
  assert.equal(active.live_channels, 3);
  assert.equal(retired.live_channels, 2);
  assert.equal(restored.session_id, original.session_id);
  assert.equal(restored.user_messages, 1);
});

test("communication channels keep independent histories and share the exact workspace across restart", async t => {
  const paths = await fixture(t);
  const mac = randomUUID();
  const body = randomUUID();
  let writes = 0;
  let reads = 0;
  const first = await host(t, configuration(paths), context => {
    const text = userText(context);
    if (text === "MAC-PRIVATE-CONTEXT") {
      return writes++ === 0
        ? { tools: [{ id: "write-shared", name: "write", arguments: { path: "shared.txt", content: "SHARED-WORKSPACE" } }] }
        : { text: "The file is ready." };
    }
    assert.ok(!JSON.stringify(context.messages).includes("MAC-PRIVATE-CONTEXT"));
    return reads++ === 0
      ? { tools: [{ id: "read-shared", name: "read", arguments: { path: "shared.txt" } }] }
      : { text: "The shared file contains SHARED-WORKSPACE." };
  });
  const request = (text: string, session_key: string) => ({
    text, session_key, channel: "text", context: "", memory_context: "Shared owner memory.",
  });
  await first.worker.request({ id: 2, method: "chat", params: request("MAC-PRIVATE-CONTEXT", mac) });
  await first.worker.request({ id: 3, method: "chat", params: request("Body hello", body) });
  assert.equal(await readFile(join(paths.workspace, "shared.txt"), "utf8"), "SHARED-WORKSPACE");
  const macStatus = await first.worker.request({ id: 4, method: "context_status", params: { session_key: mac } });
  const bodyStatus = await first.worker.request({ id: 5, method: "context_status", params: { session_key: body } });
  assert.ok(isRecord(macStatus) && isRecord(bodyStatus));
  assert.notEqual(macStatus.session_id, bodyStatus.session_id);
  await first.worker.close();
  const second = await host(t, configuration(paths), context => {
    const text = JSON.stringify(context.messages);
    if (userText(context) === "Mac return") {
      assert.ok(text.includes("MAC-PRIVATE-CONTEXT"));
      assert.ok(!text.includes("Body hello"));
    } else {
      assert.ok(text.includes("Body hello"));
      assert.ok(!text.includes("MAC-PRIVATE-CONTEXT"));
    }
    return { text: "Continuity retained." };
  });
  await second.worker.request({ id: 2, method: "chat", params: request("Mac return", mac) });
  await second.worker.request({ id: 3, method: "chat", params: request("Body return", body) });
  for (const [id, channel, original] of [[4, mac, macStatus], [5, body, bodyStatus]] as const) {
    const status = await second.worker.request({ id, method: "context_status", params: { session_key: channel } });
    assert.ok(isRecord(status));
    assert.equal(status.session_id, original.session_id);
  }
});

test("configured small model performs compaction without changing channel identity or chat model", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  config.profiles.small = { kind: "open_ai", auth_method: "api_key", api_key: "small-fixture-key" };
  config.routes.small = { profile: "small", model: "fixture-model", reasoning_effort: "low" };
  let compactCalls = 0;
  const { worker } = await host(t, config, (_context, _options, model) => {
    if (model.provider === "bobe-profile:small") {
      compactCalls++;
      return { text: "## Summary\nThe owner marker is periwinkle-19. No pending external effects." };
    }
    return { text: "Keep periwinkle-19. " + "Earlier work. ".repeat(200) };
  });
  const channel = randomUUID();
  await worker.request({ id: 2, method: "chat", params: {
    text: "Remember periwinkle-19. " + "Old context. ".repeat(200),
    session_key: channel, channel: "text", context: "",
  } });
  const before = await worker.request({ id: 3, method: "context_status", params: { session_key: channel } });
  const compacted = await worker.request({ id: 4, method: "compact", params: { session_key: channel, keep_recent_tokens: 1 } });
  const after = await worker.request({ id: 5, method: "context_status", params: { session_key: channel } });
  assert.ok(isRecord(before) && isRecord(compacted) && isRecord(after));
  assert.equal(compactCalls, 1);
  assert.equal(before.session_id, after.session_id);
  assert.equal(compacted.session_id, before.session_id);
  assert.equal(after.profile, "test");
  assert.equal(after.compactions, 1);
});
