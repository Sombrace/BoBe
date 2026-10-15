import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { setTimeout as delay } from "node:timers/promises";
import { configuration, deferred, fixture, host, userText } from "./test-support.js";
import { PriorityGate } from "./execution.js";

test("an active channel does not block another channel and remains single-flight itself", async t => {
  const paths = await fixture(t);
  const started = deferred<void>();
  const release = deferred<void>();
  const { worker } = await host(t, configuration(paths), async context => {
    if (userText(context) === "held channel") {
      started.resolve();
      await release.promise;
    }
    return { text: "complete" };
  });
  const channel = randomUUID();
  const params = { text: "held channel", channel: "text", context: "", session_key: channel };
  const first = worker.request({ id: 2, method: "chat", params });
  try {
    await started.promise;
    await worker.request({ id: 3, method: "chat", params: { ...params, text: "independent", session_key: randomUUID() } });
    await assert.rejects(worker.request({ id: 4, method: "chat", params }), /busy/);
    const status = await worker.request({ id: 5, method: "auth_status", params: { profile: "test" } });
    assert.ok(typeof status === "object" && status !== null && !Array.isArray(status) && status.is_authenticated);
  } finally {
    release.resolve();
    await first;
  }
});

for (const followUp of [false, true]) {
  test(`native pi ${followUp ? "follow-up" : "steering"} is consumed by the correlated active run`, async t => {
    const paths = await fixture(t);
    const started = deferred<void>();
    const release = deferred<void>();
    const seen: string[] = [];
    const { worker } = await host(t, configuration(paths), async context => {
      const text = userText(context);
      seen.push(text);
      if (text === "initial task") {
        started.resolve();
        await release.promise;
      }
      return { text: text === "updated direction" ? "direction applied" : "initial result" };
    });
    const running = worker.request({ id: 2, method: "chat", params: {
      text: "initial task", channel: "text", context: "",
    } });
    try {
      await started.promise;
      assert.deepEqual(await worker.request({ id: 3, method: "steer", params: {
        request_id: 2, text: "updated direction", follow_up: followUp,
      } }), { queued: true });
    } finally {
      release.resolve();
      await running;
    }
    assert.ok(seen.includes("updated direction"));
    await assert.rejects(worker.request({ id: 4, method: "steer", params: {
      request_id: 2, text: "must not leak into a later turn", follow_up: followUp,
    } }), /no longer accepting/);
  });
}

test("background admission reserves an interactive slot and queued cancellation releases no capacity", async () => {
  const gate = new PriorityGate(4, 1);
  const releases = await Promise.all([0, 1, 2].map(() => gate.acquire("background")));
  const cancel = new AbortController();
  const waiting = gate.acquire("background", cancel.signal);
  const rejected = assert.rejects(waiting);
  const foreground = await gate.acquire("interactive");
  assert.deepEqual(gate.snapshot(), { active: 4, queued: 1, background: 3, capacity: 4 });
  cancel.abort();
  await rejected;
  foreground();
  for (const release of releases) release();
  assert.deepEqual(gate.snapshot(), { active: 0, queued: 0, background: 0, capacity: 4 });
});

test("output ceilings reach the real provider request and oversized input never reaches it", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  config.workload_budgets.chat = {
    max_prompt_tokens: 32000, max_output_tokens: 256, wall_clock: { secs: 30, nanos: 0 },
  };
  let calls = 0;
  const { worker } = await host(t, config, (_context, options, model) => {
    calls++;
    assert.equal(model.maxTokens, 256);
    assert.equal(options?.maxTokens, 256);
    return { text: "bounded" };
  });
  await worker.request({ id: 2, method: "chat", params: { text: "short input", channel: "text", context: "" } });
  await assert.rejects(worker.request({ id: 3, method: "chat", params: {
    text: "x".repeat(160_000), channel: "text", context: "", session_key: randomUUID(),
  } }), /context|budget|compact/i);
  assert.equal(calls, 1);
});

test("a runtime-selected background turn limit stops an otherwise continuing tool loop", async t => {
  const paths = await fixture(t);
  let calls = 0;
  const { worker } = await host(t, configuration(paths), () => {
    calls++;
    return { tools: [{ id: `listing-${calls}`, name: "ls", arguments: { path: "." } }] };
  });
  await assert.rejects(worker.request({ id: 2, method: "background", params: {
    job_id: randomUUID(), title: "limited task", task: "list", context: "", memory_context: "", attempt: 1,
    route: { profile: "test", model: "fixture-model", reasoning_effort: "low" }, budget: { timeout_seconds: 10, max_turns: 2 },
  } }), /turn limit/);
  assert.equal(calls, 2);
});

test("one foreground root and two background roots share an aggregate four-call inference limit", async t => {
  const paths = await fixture(t);
  const release = deferred<void>();
  const firstLeaf = deferred<void>();
  const parents = new Set<string>();
  let active = 0;
  let peak = 0;
  const { worker } = await host(t, configuration(paths), async context => {
    const text = userText(context);
    if (text.startsWith("LEAF ")) {
      active++;
      peak = Math.max(peak, active);
      firstLeaf.resolve();
      try { await release.promise; return { text: "settled" }; }
      finally { active--; }
    }
    if (parents.has(text)) return { text: "complete" };
    parents.add(text);
    return { tools: [0, 1].map(group => ({
      id: `delegate-${group}`, name: "subagent", arguments: {
        mode: "parallel", tasks: [0, 1, 2, 3].map(index => ({ task: `LEAF ${text} ${group} ${index}` })),
      },
    })) };
  });
  const route = { profile: "test", model: "fixture-model", reasoning_effort: "low" };
  const runs = [
    worker.request({ id: 2, method: "chat", params: { text: "FOREGROUND", channel: "text", context: "" } }),
    ...[3, 4].map(id => worker.request({ id, method: "background", params: {
      job_id: randomUUID(), title: `Root ${id}`, task: `BACKGROUND ${id}`,
      context: "", memory_context: "", attempt: 1, route, budget: { timeout_seconds: 1800, max_turns: 64 },
    } })),
  ];
  try {
    await firstLeaf.promise;
    await delay(200);
  } finally { release.resolve(); }
  await Promise.all(runs);
  assert.ok(peak <= 4, `observed ${peak} simultaneous provider calls`);
  assert.equal(active, 0);
});
