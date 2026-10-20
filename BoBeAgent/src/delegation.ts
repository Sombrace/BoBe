import { Type, type Static } from "typebox";
import { defineTool, type ToolDefinition } from "@earendil-works/pi-coding-agent";
import type { Usage } from "@earendil-works/pi-ai";
import { assertActive, rejections } from "./errors.ts";
import { addUsage } from "./events.ts";
import { json, taskBudgetSchema, type JsonObject } from "./protocol.ts";

const taskSchema = Type.Object({
  agent: Type.Optional(Type.String({ minLength: 1, maxLength: 100 })),
  task: Type.String({ minLength: 1 }),
  context: Type.Optional(Type.String()),
  budget: Type.Optional(taskBudgetSchema),
}, { additionalProperties: false });
export type DelegateTask = Static<typeof taskSchema>;

const parameters = Type.Object({
  mode: Type.Enum(["parallel", "chain"]),
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

export interface DelegateOutcome {
  result: DelegateResult;
  usage: Usage | undefined;
}

interface DelegateRunner {
  run(task: DelegateTask, signal?: AbortSignal): Promise<DelegateOutcome>;
}

async function boundedParallel<T, R>(
  items: readonly T[], concurrency: number, execute: (item: T) => Promise<R>, signal?: AbortSignal,
): Promise<R[]> {
  const queue = items.entries();
  const results: R[] = [];
  const workers = Array.from({ length: Math.min(items.length, concurrency) }, async () => {
    for (const [index, item] of queue) {
      assertActive(signal);
      results[index] = await execute(item);
    }
  });
  const [failure] = rejections(await Promise.allSettled(workers));
  if (failure !== undefined) throw failure;
  return results;
}

export function subagentTool(runner: DelegateRunner): ToolDefinition {
  return defineTool({
    name: "subagent",
    label: "Scoped subagent",
    description: "Delegate bounded work to isolated pi sessions with the same tools, skills, workspace, provider, and owner authority. Always provide mode and a tasks list. mode=parallel runs independent tasks concurrently (up to eight tasks, four active); a one-item list runs one child. mode=chain runs tasks in order, replacing {previous} with the prior result. Put each task's agent, task and context inside the list. This call waits for all children; cancellation cancels and settles them.",
    parameters,
    execute: async (_id, args, signal) => {
      let outcomes: DelegateOutcome[];
      if (args.mode === "parallel") {
        outcomes = await boundedParallel(args.tasks, 4, task => runner.run(task, signal), signal);
      } else {
        outcomes = [];
        let previous = "";
        for (const task of args.tasks) {
          assertActive(signal);
          const outcome = await runner.run({ ...task, task: task.task.replaceAll("{previous}", previous) }, signal);
          outcomes.push(outcome);
          if (!outcome.result.success) break;
          previous = outcome.result.text;
        }
      }
      const results = outcomes.map(outcome => outcome.result);
      const usage = outcomes.reduce<Usage | undefined>(
        (total, outcome) => outcome.usage ? addUsage(total, outcome.usage) : total, undefined);
      const value: JsonObject = { mode: args.mode, results: json(results) };
      return {
        content: [{ type: "text", text: JSON.stringify(value) }],
        details: value,
        isError: results.some(result => !result.success),
        ...(usage ? { usage } : {}),
      };
    },
  });
}

export function boundedOutput(text: string): { text: string; truncated: boolean } {
  const limit = 50 * 1024;
  if (Buffer.byteLength(text) <= limit) return { text, truncated: false };
  const { read } = new TextEncoder().encodeInto(text, new Uint8Array(limit));
  return { text: `${text.slice(0, read)}\n[Delegated output truncated; narrow the task for more detail.]`, truncated: true };
}
