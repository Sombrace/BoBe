import assert from "node:assert/strict";
import test from "node:test";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { AGENT_LAYOUT } from "./layout.js";
import {
  HARNESS_VERSION, MAX_LINE_BYTES, PROTOCOL_VERSION, abortSchema, chatSchema,
  domainResultSchema, emptySchema, initializeSchema, object, parseEnvelope, validate,
} from "./protocol.js";

test("Rust can mirror the exact host-contract-v1 fixture and authority filenames", async () => {
  const parsed: unknown = JSON.parse(await readFile(new URL("../fixtures/host-contract-v1.json", import.meta.url), "utf8"));
  const fixture = object(parsed);
  assert.equal(fixture.protocol_version, PROTOCOL_VERSION);
  assert.equal(fixture.harness_version, HARNESS_VERSION);
  assert.equal(fixture.max_line_bytes, MAX_LINE_BYTES);
  assert.deepEqual(fixture.layout, Object.fromEntries(
    Object.entries(AGENT_LAYOUT).map(([name, path]) => [name, `agent/${path}`]),
  ));
  const auth = object(fixture.profile_auth_example);
  assert.equal(auth.provider_id, `bobe-profile:${auth.profile_id}`);
  assert.equal(typeof auth.provider_id, "string");
  if (typeof auth.provider_id !== "string") throw new Error("Invalid fixture provider ID");
  const digest = createHash("sha256").update(auth.provider_id).digest("hex");
  assert.equal(auth.path, `agent/${AGENT_LAYOUT.credentials}/${digest}.json`);
  for (const [name, schema] of [
    ["initialize_request", initializeSchema], ["chat_request", chatSchema],
    ["abort_request", abortSchema], ["shutdown_request", emptySchema],
  ] as const) {
    const request = parseEnvelope(fixture[name]);
    assert.ok("method" in request);
    validate(schema, request.params, name);
  }
  const initial = object(object(fixture.initialize_request).params);
  for (const name of ["execution", "workload_budgets", "capabilities"]) {
    const incomplete = { ...initial };
    delete incomplete[name];
    assert.throws(() => validate(initializeSchema, incomplete, "initialization requires owner policy"));
  }
  assert.throws(() => validate(initializeSchema, { ...initial, capability_instructions: "Prose." },
    "initialization carries capability facts, not behavior prose"));
  for (const name of ["domain_tool_response", "domain_tool_failed_response"]) {
    const response = parseEnvelope(fixture[name]);
    assert.ok("result" in response);
    validate(domainResultSchema, response.result, name);
  }
  const delta = object(object(fixture.message_delta).params);
  const complete = object(object(fixture.message_complete).params);
  assert.equal(delta.request_id, complete.request_id);
  assert.equal(object(delta.delta).message_id, object(complete.delta).message_id);
});
