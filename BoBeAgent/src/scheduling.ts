import { createHash } from "node:crypto";
import { Type } from "typebox";
import { defineTool, type ToolDefinition } from "@earendil-works/pi-coding-agent";
import { object, taskBudgetSchema } from "./protocol.js";
import type { JobControl } from "./background.js";

export function scheduleTool(control: JobControl): ToolDefinition {
  const identity = { schedule_id: Type.String({ format: "uuid" }) };
  return defineTool({
    name: "bobe_schedule",
    label: "Plan future work",
    description: "Create or manage durable future work in this same BoBe authority. Use an RFC 3339 at time and optional every_seconds for repetition. Missed intervals produce at most one catch-up, never a replay burst; an active occurrence cannot overlap its successor. Supply complete task guidance. Pause stops future and queued occurrences, not an already running task. Use background task controls to stop current work. Goal-bound schedules remain tied to that goal.",
    parameters: Type.Union([
      Type.Object({
        action: Type.Literal("create"),
        title: Type.String({ minLength: 1, maxLength: 160 }),
        task: Type.String({ minLength: 1, maxLength: 32768 }),
        context: Type.String({ maxLength: 12000 }),
        at: Type.String({ format: "date-time" }),
        every_seconds: Type.Optional(Type.Integer({ minimum: 1, maximum: 31536000 })),
        goal_id: Type.Optional(Type.String({ format: "uuid" })),
        budget: Type.Optional(taskBudgetSchema),
      }, { additionalProperties: false }),
      Type.Object({ action: Type.Literal("list") }, { additionalProperties: false }),
      Type.Object({
        action: Type.Union([Type.Literal("get"),Type.Literal("pause"),Type.Literal("resume"),Type.Literal("cancel")]),
        ...identity,
      }, { additionalProperties: false }),
      Type.Object({
        action: Type.Literal("reschedule"), ...identity,
        generation: Type.Integer({ minimum: 1 }), at: Type.String({ format: "date-time" }),
      }, { additionalProperties: false }),
    ]),
    execute: async (callId, args, signal) => {
      const { action, ...parameters } = args;
      const source = createHash("sha256").update(`${control.invocationScope()}:${callId}`).digest("hex");
      const value = await control.call(action, object(parameters), source, signal);
      return { content: [{ type: "text", text: JSON.stringify(value) }], details: value, structuredContent: value };
    },
  });
}
