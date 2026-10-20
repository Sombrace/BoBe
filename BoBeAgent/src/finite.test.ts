import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { getCurrentSystemMessage, type TranscriptContext } from "@earendil-works/pi-ai";
import { configuration, fixture, host, userText } from "./test-support.ts";
import { isRecord, type Json } from "./protocol.ts";

test("auxiliary structured-result tool is a real SDK tool and does not enter the main transcript", async t => {
  const paths = await fixture(t);
  const contexts: TranscriptContext[] = [];
  const { worker } = await host(t, configuration(paths), context => {
    contexts.push(structuredClone(context));
    return { tools: [{ id: "submit-1", name: "bobe_submit_result", arguments: { output: { goals: ["bounded"] } } }] };
  });
  const pointer = await readFile(join(paths.root, "agent/session.json"), "utf8");
  const result = await worker.request({
    id: 2, method: "auxiliary",
    params: { class: "small", job: { job_id: "job-1", kind: "extract", instructions: "Extract goals.", input: {} }, context: "finite owner context" },
  });
  assert.deepEqual(result, { job_id: "job-1", output: { goals: ["bounded"] }, error: null });
  const sections = getCurrentSystemMessage(contexts[0]?.messages ?? [])?.sections ?? {};
  assert.ok(sections.preamble?.startsWith("You are a bounded operation of BoBe, not another companion."));
  for (const companion of ["bobe_persona", "bobe_capabilities", "bobe_memory"]) assert.equal(sections[companion], undefined);
  assert.equal(await readFile(join(paths.root, "agent/session.json"), "utf8"), pointer);
  const parsed: unknown = JSON.parse(pointer);
  assert.ok(isRecord(parsed) && typeof parsed.file === "string");
  assert.equal((await readFile(join(paths.root, "agent", parsed.file), "utf8")).includes("finite owner context"), false);
});

test("Rust auxiliary jobs receive the submitted result value unchanged", async t => {
  for (const result of [
    { kind: "proactive_activation", value: { activate: true, reason: "The owner asked for this reminder." } },
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

test("auxiliary accepts the result as JSON text, fenced or bare, and reports anything else as failure", async t => {
  const cases: [string, unknown][] = [
    ['{"activate":false,"reason":""}', { activate: false, reason: "" }],
    ['```json\n{"ok":true}\n```', { ok: true }],
    ["[]", []],
    ["Not structured output", undefined],
    ["", undefined],
  ];
  for (const [text, expected] of cases) {
    await t.test(text || "empty", async t => {
      const paths = await fixture(t);
      const { worker } = await host(t, configuration(paths), () => ({ text }));
      const result = await worker.request({
        id: 2, method: "auxiliary",
        params: { class: "consolidate", job: { job_id: "job-2", kind: "consolidate", instructions: "Produce result.", input: {} }, context: "" },
      });
      assert.ok(isRecord(result));
      if (expected === undefined) {
        assert.equal(typeof result.error, "string");
        assert.equal(result.output, null);
      } else {
        assert.equal(result.error, null);
        assert.deepEqual(result.output, expected);
      }
    });
  }
});

test("auxiliary keeps a valid submission when another submit call fails", async t => {
  const value = { activate: true, reason: "The owner asked for this reminder." };
  const submit = (id: string, args: { [key: string]: Json }) => ({ id, name: "bobe_submit_result", arguments: args });
  type Reply = { text?: string; tools?: ReturnType<typeof submit>[] };
  const cases: [string, (results: number) => Reply][] = [
    ["repeated in one reply", results => results === 0
      ? { tools: [submit("first", { output: value }), submit("again", { output: value })] } : { text: "" }],
    ["corrected after a malformed call", results => results === 0 ? { tools: [submit("flat", value)] }
      : results === 1 ? { tools: [submit("wrapped", { output: value })] } : { text: "" }],
  ];
  for (const [name, reply] of cases) {
    await t.test(name, async t => {
      const paths = await fixture(t);
      const { worker } = await host(t, configuration(paths), context =>
        reply(context.messages.filter(message => message.role === "toolResult").length));
      assert.deepEqual(await worker.request({
        id: 2, method: "auxiliary",
        params: { class: "small", job: { job_id: "job-3", kind: "proactive_activation", instructions: "Decide.", input: {} }, context: "" },
      }), { job_id: "job-3", output: value, error: null });
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
