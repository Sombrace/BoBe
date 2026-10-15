import { Type, type Static } from "typebox";
import { defineTool, type ToolDefinition } from "@earendil-works/pi-coding-agent";
import { assertActive } from "./errors.js";
import { json, taskBudgetSchema, type JsonObject } from "./protocol.js";

const taskSchema = Type.Object({
  agent: Type.Optional(Type.String({ minLength: 1, maxLength: 100 })),
  task: Type.String({ minLength: 1 }),
  context: Type.Optional(Type.String()),
  budget: Type.Optional(taskBudgetSchema),
}, { additionalProperties: false });
export type DelegateTask = Static<typeof taskSchema>;

const parameters = Type.Object({
  mode: Type.Union([Type.Literal("parallel"), Type.Literal("chain")]),
  tasks: Type.Array(taskSchema, { minItems: 1, maxItems: 8 }),
}, { additionalProperties: false });

export interface DelegateResult {
  agent: string;
  text: string;
  output_tokens: number;
  success: boolean;
  error: string | null;
  truncated: boolean;
}

interface DelegateRunner {
  run(task: DelegateTask, signal?: AbortSignal): Promise<DelegateResult>;
}

async function boundedParallel<T, R>(
  items: readonly T[], concurrency: number, execute: (item: T) => Promise<R>, signal?: AbortSignal,
): Promise<R[]> {
  let cursor = 0;
  const results = new Map<number, R>();
  const workers = Array.from({ length: Math.min(items.length, concurrency) }, async () => {
    while (cursor < items.length) {
      assertActive(signal);
      const index = cursor++;
      const item = items[index];
      if (item !== undefined) results.set(index, await execute(item));
    }
  });
  const settled = await Promise.allSettled(workers);
  const failed = settled.find(result => result.status === "rejected");
  if (failed?.status === "rejected") throw failed.reason;
  return items.map((_, index) => {
    const result = results.get(index);
    if (result === undefined) throw new Error("Delegated task did not settle");
    return result;
  });
}

export function subagentTool(runner: DelegateRunner): ToolDefinition {
  return defineTool({
    name: "subagent",
    label: "Scoped subagent",
    description: "Delegate bounded work to isolated pi sessions with the same tools, skills, workspace, provider, and owner authority. Always provide mode and a tasks list. mode=parallel runs independent tasks concurrently (up to eight tasks, four active); a one-item list runs one child. mode=chain runs tasks in order, replacing {previous} with the prior result. Put each task's agent, task and context inside the list. This call waits for all children; cancellation cancels and settles them.",
    parameters,
    execute: async (_id, args, signal) => {
      let results: DelegateResult[];
      if (args.mode === "parallel") {
        results = await boundedParallel(args.tasks, 4, task => runner.run(task, signal), signal);
      } else {
        results = [];
        let previous = "";
        for (const task of args.tasks) {
          assertActive(signal);
          const result = await runner.run({ ...task, task: task.task.replaceAll("{previous}", previous) }, signal);
          results.push(result);
          if (!result.success) break;
          previous = result.text;
        }
      }
      const value: JsonObject = { mode: args.mode, results: json(results) };
      return {
        content: [{ type: "text", text: JSON.stringify(value) }],
        details: value,
        structuredContent: value,
        isError: results.some(result => !result.success),
      };
    },
  });
}

export function boundedOutput(text: string): { text: string; truncated: boolean } {
  const limit = 50 * 1024;
  const bytes = Buffer.from(text);
  if (bytes.length <= limit) return { text, truncated: false };
  let end = limit;
  while (end > 0 && (bytes[end]! & 0xc0) === 0x80) end--;
  return {
    text: `${bytes.subarray(0, end).toString("utf8")}\n[Delegated output truncated; narrow the task for more detail.]`,
    truncated: true,
  };
}
