import { Type, type Static, type TSchema } from "typebox";
import { Compile, type Validator } from "typebox/compile";

export const PROTOCOL_VERSION = 1;
export const HARNESS_VERSION = "1.4.0";
export const MAX_LINE_BYTES = 16 * 1024 * 1024;

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
export type JsonObject = { [key: string]: Json };

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function isJson(value: unknown): value is Json {
  if (value === null || typeof value === "string" || typeof value === "boolean") return true;
  if (typeof value === "number") return Number.isFinite(value);
  if (Array.isArray(value)) return value.every(isJson);
  return isRecord(value) && Object.values(value).every(isJson);
}

export function json(value: unknown): Json {
  if (!isJson(value)) throw new Error("Value is not JSON");
  return value;
}

export function object(value: unknown): JsonObject {
  if (!isRecord(value) || !isJson(value)) throw new Error("Expected a JSON object");
  return value;
}

const validators = new WeakMap<TSchema, Validator>();

function compiled<T extends TSchema>(schema: T): Validator<{}, T> {
  return validators.getOrInsertComputed(schema, key => Compile(key)) as Validator<{}, T>;
}

export function validate<T extends TSchema>(schema: T, value: unknown, label: string): Static<T> {
  if (!compiled(schema).Check(value)) throw new Error(`Invalid ${label}`);
  return value;
}

const strict = { additionalProperties: false };
export const toolNames = ["read", "bash", "edit", "write", "grep", "find", "ls", "subagent", "background_jobs"] as const;
const toolNameSchema = Type.Enum(toolNames);
const text = Type.String();
const nonempty = Type.String({ minLength: 1 });
const nullableText = Type.Union([text, Type.Null()]);
const strings = Type.Array(text);
const thinkingSchema = Type.Enum(["off", "minimal", "low", "medium", "high", "xhigh", "max"]);
export type Thinking = Static<typeof thinkingSchema>;

const profileSchema = Type.Object({
  kind: Type.Enum(["copilot", "open_ai", "anthropic", "deepseek", "open_router", "rail_relay", "open_ai_compatible"]),
  base_url: Type.Optional(nullableText),
  api_key: Type.Optional(nullableText),
  organization: Type.Optional(nullableText),
  offline: Type.Optional(Type.Boolean()),
  local_runtime: Type.Optional(Type.Union([Type.Literal("ollama"), Type.Null()])),
  auth_method: Type.Enum(["api_key", "oauth"]),
}, strict);
export type Profile = Static<typeof profileSchema>;

const routeSchema = Type.Object({
  profile: nonempty,
  model: Type.Union([nonempty, Type.Null()]),
  reasoning_effort: Type.Union([thinkingSchema, Type.Null()]),
}, strict);
export type Route = Static<typeof routeSchema>;

const serverSchema = Type.Object({
  command: nonempty,
  args: strings,
  env: Type.Record(text, text),
  timeout_ms: Type.Integer({ minimum: 1, maximum: 600_000 }),
  excluded_tools: strings,
}, strict);
const mcpSchema = Type.Object({ servers: Type.Record(nonempty, serverSchema) }, strict);
export type McpConfig = Static<typeof mcpSchema>;

const domainToolSchema = Type.Object({
  name: Type.String({ pattern: "^bobe_[A-Za-z0-9_]+$" }),
  description: nonempty,
  parameters: Type.Record(text, Type.Unknown()),
}, strict);

const personaSchema = Type.Object({ souls: strings, profiles: strings }, strict);
export type Persona = Static<typeof personaSchema>;

const workloadBudgetSchema = Type.Object({
  wall_clock: Type.Object({
    secs: Type.Integer({ minimum: 1 }), nanos: Type.Integer({ minimum: 0, maximum: 999_999_999 }),
  }, strict),
  max_prompt_tokens: Type.Integer({ minimum: 128 }),
  max_output_tokens: Type.Integer({ minimum: 64 }),
}, strict);

export const initializeSchema = Type.Object({
  execution: Type.Object({
    model_concurrency: Type.Integer({ minimum: 2, maximum: 32 }),
    foreground_reserve: Type.Integer({ minimum: 1, maximum: 31 }),
    tool_concurrency: Type.Integer({ minimum: 1, maximum: 64 }),
    live_channels: Type.Integer({ minimum: 1, maximum: 32 }),
    idle_channel_seconds: Type.Integer({ minimum: 30, maximum: 86400 }),
    task_timeout_seconds: Type.Integer({ minimum: 1, maximum: 86400 }),
    task_max_turns: Type.Integer({ minimum: 1, maximum: 512 }),
  }, strict),
  workload_budgets: Type.Object({
    chat: workloadBudgetSchema, goals: workloadBudgetSchema, small: workloadBudgetSchema,
    consolidate: workloadBudgetSchema,
  }, strict),
  protocol_version: Type.Literal(PROTOCOL_VERSION),
  data_root: nonempty,
  workspace: nonempty,
  skill_directories: strings,
  profiles: Type.Record(nonempty, profileSchema),
  routes: Type.Object({
    chat: routeSchema,
    goals: routeSchema,
    consolidate: routeSchema,
    small: Type.Optional(routeSchema),
  }, strict),
  tools: Type.Array(domainToolSchema),
  capabilities: Type.Object({ memory: Type.Boolean(), goals: Type.Boolean() }, strict),
  mcp: mcpSchema,
  enabled_tools: Type.Array(toolNameSchema, { uniqueItems: true }),
}, strict);
export type Initialize = Static<typeof initializeSchema>;

export const chatSchema = Type.Object({
  session_key: Type.Optional(Type.String({ format: "uuid" })),
  priority: Type.Optional(Type.Enum(["interactive", "background"])),
  text,
  channel: Type.Enum(["text", "voice"]),
  context: text,
  memory_context: Type.Optional(text),
  persona: Type.Optional(personaSchema),
}, strict);
export type Chat = Static<typeof chatSchema>;

export const auxiliarySchema = Type.Object({
  class: Type.Enum(["consolidate", "small"]),
  job: Type.Object({
    job_id: nonempty,
    kind: nonempty,
    instructions: text,
    input: Type.Record(text, Type.Unknown()),
  }, strict),
  context: text,
}, strict);
export type Auxiliary = Static<typeof auxiliarySchema>;

export const reloadSchema = Type.Object({
  mcp: Type.Optional(mcpSchema),
  skill_directories: Type.Optional(strings),
}, strict);
export const emptySchema = Type.Object({}, strict);
export const profileRequestSchema = Type.Object({ profile: nonempty }, strict);
export const probeSchema = Type.Object({
  route: Type.Optional(routeSchema),
}, strict);
export const authSubmitSchema = Type.Object({ profile: nonempty, value: text }, strict);
export const abortSchema = Type.Object({ request_id: Type.Integer({ minimum: 1 }) }, strict);
export const steerSchema = Type.Object({
  request_id: Type.Integer({ minimum: 1 }),
  text: Type.String({ minLength: 1, maxLength: 32_768 }),
  follow_up: Type.Boolean(),
}, strict);
export const mcpToolsSchema = Type.Object({ server_name: nonempty }, strict);
export const compactSchema = Type.Object({
  session_key: Type.Optional(Type.String({ format: "uuid" })),
  keep_recent_tokens: Type.Optional(Type.Union([
    Type.Integer({ minimum: 1, maximum: 64_000 }), Type.Null(),
  ])),
}, strict);
export const contextStatusSchema = Type.Object({
  session_key: Type.Optional(Type.String({ format: "uuid" })),
}, strict);
export const taskBudgetSchema = Type.Object({
  timeout_seconds: Type.Optional(Type.Integer({ minimum: 1, maximum: 86400 })),
  max_turns: Type.Optional(Type.Integer({ minimum: 1, maximum: 512 })),
}, strict);
export type TaskBudget = Static<typeof taskBudgetSchema>;
export const backgroundSchema = Type.Object({
  job_id: Type.String({ format: "uuid" }),
  title: nonempty,
  task: nonempty,
  context: text,
  memory_context: text,
  persona: personaSchema,
  attempt: Type.Integer({ minimum: 1 }),
  route: routeSchema,
  budget: Type.Object({
    timeout_seconds: Type.Integer({ minimum: 1, maximum: 86400 }),
    max_turns: Type.Integer({ minimum: 1, maximum: 512 }),
  }, strict),
}, strict);
export type BackgroundRequest = Static<typeof backgroundSchema>;
export const domainResultSchema = Type.Union([
  Type.Object({ success: Type.Literal(true), value: Type.Unknown() }, strict),
  Type.Object({
    success: Type.Literal(false),
    value: Type.Object({ ok: Type.Literal(false), error: text }, strict),
  }, strict),
]);

export type Delta =
  | { type: "message_delta"; message_id: string; text: string }
  | { type: "message_complete"; message_id: string; content: string; output_tokens: number }
  | { type: "tool_start"; id: string; name: string }
  | { type: "tool_complete"; id: string; name: string; success: boolean }
  | { type: "error"; message: string }
  | { type: "done" };

export type AuthPhase =
  | { phase: "preparing" | "completed" | "canceled" }
  | { phase: "awaiting_user" | "polling"; url: string; code: string }
  | { phase: "awaiting_input"; message: string; password: boolean }
  | { phase: "failed"; message: string };

export type Notification =
  | { method: "event"; params: { request_id: number; delta: Delta } }
  | { method: "tool_activity"; params: { request_id: number; name: string } }
  | { method: "auth_event"; params: { profile: string; phase: AuthPhase } };

export type Request = { id: number; method: string; params: JsonObject };
type Reply = { id: number; result: Json } | { id: number; error: { message: string } };

export function parseEnvelope(value: unknown): Request | Reply {
  const envelope = object(value);
  const id = envelope.id;
  if (typeof id !== "number" || !Number.isSafeInteger(id) || id === 0) {
    throw new Error("RPC message requires a nonzero safe integer id");
  }
  if ("method" in envelope) {
    if (id < 1 || typeof envelope.method !== "string" || !envelope.method ||
      Object.keys(envelope).some(key => !["id", "method", "params"].includes(key))) {
      throw new Error("Invalid RPC request");
    }
    return { id, method: envelope.method, params: object(envelope.params) };
  }
  if (id > -1) throw new Error("Worker callback response requires a negative id");
  if ("result" in envelope && Object.keys(envelope).length === 2) {
    return { id, result: json(envelope.result) };
  }
  if ("error" in envelope && Object.keys(envelope).length === 2) {
    const error = object(envelope.error);
    if (typeof error.message === "string" && Object.keys(error).length === 1) {
      return { id, error: { message: error.message } };
    }
  }
  throw new Error("Invalid RPC response");
}
