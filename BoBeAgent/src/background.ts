import { hash } from "node:crypto";
import { defineTool, type ToolDefinition } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { object, taskBudgetSchema, type Json, type JsonObject } from "./protocol.ts";
import { assertActive } from "./errors.ts";

export interface JobControl {
  invocationScope(): string;
  call(action: string, arguments_: JsonObject, sourceKey: string | undefined, signal?: AbortSignal): Promise<Json>;
}

export function sourceKey(control: JobControl, callId: string): string {
  return hash("sha256", `${control.invocationScope()}:${callId}`);
}

export function backgroundTools(control: JobControl, allowStart = true): ToolDefinition[] {
  const result = (value: Json) => ({
    content: [{ type: "text" as const, text: JSON.stringify(value) }],
    details: value,
  });
  return [
    ...(allowStart ? [defineTool({
      name: "background_task",
      label: "Start background work",
      description: "Start one durable, bounded task without waiting for its result. Rust owns its job identity and lifecycle. It may continue after this reply; its result returns to this same companion when idle. Supply complete context, not another persona. Do not blindly repeat interrupted effects. Use status to inspect and cancel to stop.",
      parameters: Type.Object({
        title: Type.String({ minLength: 1, maxLength: 160 }),
        task: Type.String({ minLength: 1, maxLength: 32768 }),
        context: Type.String({ maxLength: 16384 }),
        budget: Type.Optional(taskBudgetSchema),
      }, { additionalProperties: false }),
      execute: async (callId, args, signal) => {
        assertActive(signal);
        return result(await control.call("start", object(args), sourceKey(control, callId), signal));
      },
    })] : []),
    defineTool({
      name: "background_tasks",
      label: "List background work",
      description: "List durable background tasks and their actual status/results. Running work is not completed work.",
      parameters: Type.Object({}, { additionalProperties: false }),
      execute: async (_id, _args, signal) => result(await control.call("list", {}, undefined, signal)),
    }),
    ...(["get", "cancel"] as const).map(action => defineTool({
      name: action === "get" ? "background_task_status" : "background_task_cancel",
      label: action === "get" ? "Inspect background task" : "Cancel background task",
      description: action === "get"
        ? "Read one task's durable state, result or interruption reason."
        : "Request cancellation of one background task. Queued work stops immediately; running work must settle before it reports cancelled.",
      parameters: Type.Object({ job_id: Type.String({ format: "uuid" }) }, { additionalProperties: false }),
      execute: async (_id, args, signal) => result(await control.call(action, args, undefined, signal)),
    })),
  ];
}
