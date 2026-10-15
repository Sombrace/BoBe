import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { configuration, fixture, host, userText } from "./test-support.js";
import { isRecord } from "./protocol.js";

test("auxiliary structured-result tool is a real SDK tool and does not enter the main transcript", async t => {
  const paths = await fixture(t);
  const { worker } = await host(t, configuration(paths), () => ({
    tools: [{ id: "submit-1", name: "bobe_submit_result", arguments: { output: { goals: ["bounded"] } } }],
  }));
  const pointer = await readFile(join(paths.root, "agent/session.json"), "utf8");
  const result = await worker.request({
    id: 2, method: "auxiliary",
    params: { class: "small", job: { job_id: "job-1", kind: "extract", instructions: "Extract goals.", input: {} }, context: "finite owner context" },
  });
  assert.deepEqual(result, { job_id: "job-1", output: { goals: ["bounded"] }, error: null });
  assert.equal(await readFile(join(paths.root, "agent/session.json"), "utf8"), pointer);
  const parsed: unknown = JSON.parse(pointer);
  assert.ok(isRecord(parsed) && typeof parsed.file === "string");
  assert.equal((await readFile(join(paths.root, "agent", parsed.file), "utf8")).includes("finite owner context"), false);
});

test("Rust auxiliary jobs preserve result values rather than an extra output envelope", async t => {
  for (const result of [
    { kind: "proactive_activation", value: { activate: true, context: "The owner asked for this reminder." } },
    { kind: "conversation_summary", value: "The owner plans to practise Spanish this week." },
    { kind: "consolidate", value: { body: "# Memory\n\nA durable fact." } },
  ]) {
    await t.test(result.kind, async t => {
      const paths = await fixture(t);
      const { worker } = await host(t, configuration(paths), () => ({
        tools: [{ id: "submit-value", name: "bobe_submit_result", arguments: { output: result.value } }],
      }));
      assert.deepEqual(await worker.request({
        id: 2, method: "auxiliary",
        params: {
          class: result.kind === "consolidate" ? "consolidate" : "small",
          job: { job_id: "rust-job", kind: result.kind, instructions: "Submit the requested result.", input: {} },
          context: "",
        },
      }), { job_id: "rust-job", output: result.value, error: null });
    });
  }
});

test("auxiliary accepts a validated result envelope and reports malformed or missing output as failure", async t => {
  for (const text of ['{"output":{"ok":true}}', '{"ok":true}', "Not structured output", "[]", ""]) {
    await t.test(text || "empty", async t => {
      const paths = await fixture(t);
      const { worker } = await host(t, configuration(paths), () => ({ text }));
      const result = await worker.request({
        id: 2, method: "auxiliary",
        params: { class: "consolidate", job: { job_id: "job-2", kind: "consolidate", instructions: "Produce result.", input: {} }, context: "" },
      });
      assert.ok(isRecord(result));
      if (text.startsWith('{"output":')) {
        assert.equal(result.error, null);
        assert.deepEqual(result.output, { ok: true });
      } else {
        assert.equal(typeof result.error, "string");
        assert.equal(result.output, null);
      }
    });
  }
});

test("probe requires streamed text, a real echo tool call, and matching final output", async t => {
  const paths = await fixture(t);
  let calls = 0;
  const { worker } = await host(t, configuration(paths), context => {
    const nonce = userText(context).match(/value ([0-9a-f-]+),/)?.[1];
    assert.ok(nonce);
    const seenResult = context.messages.some(message => message.role === "toolResult");
    calls++;
    return seenResult ? { text: nonce }
      : { tools: [{ id: "probe-1", name: "bobe_probe_echo", arguments: { value: nonce } }] };
  });
  assert.deepEqual(await worker.request({ id: 2, method: "probe", params: {} }), {
    model_execution: true, streaming: true, tool_execution: true, error: null,
  });
  assert.equal(calls, 2);
});

test("probe cannot claim tool success from an ordinary assistant response", async t => {
  const paths = await fixture(t);
  const { worker } = await host(t, configuration(paths), () => ({ text: "Everything works" }));
  const result = await worker.request({ id: 2, method: "probe", params: {} });
  assert.ok(isRecord(result));
  assert.equal(result.tool_execution, false);
  assert.equal(typeof result.error, "string");
});

test("profile probe tests the selected provider without changing channel routes", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  config.profiles.other = { kind: "open_ai", auth_method: "api_key", api_key: "other-fixture" };
  const seenProviders: string[] = [];
  const { worker } = await host(t, config, (context, _options, model) => {
    seenProviders.push(model.provider);
    const nonce = userText(context).match(/value ([0-9a-f-]+),/)?.[1];
    assert.ok(nonce);
    return context.messages.some(message => message.role === "toolResult")
      ? { text: nonce } : { tools: [{ id: "test-echo", name: "bobe_probe_echo", arguments: { value: nonce } }] };
  });
  const pointer = await readFile(join(paths.root, "agent/session.json"), "utf8");
  assert.deepEqual(await worker.request({
    id: 2, method: "probe",
    params: { route: { profile: "other", model: "fixture-model", reasoning_effort: "low" } },
  }), { model_execution: true, streaming: true, tool_execution: true, error: null });
  assert.deepEqual(seenProviders, ["bobe-profile:other", "bobe-profile:other"]);
  assert.equal(await readFile(join(paths.root, "agent/session.json"), "utf8"), pointer);
  assert.equal(config.routes.chat.profile, "test");
});
