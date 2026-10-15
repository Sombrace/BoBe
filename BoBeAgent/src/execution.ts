import { AsyncLocalStorage } from "node:async_hooks";
import {
  createAssistantMessageEventStream, type Api, type AssistantMessage,
  type AssistantMessageEventStream, type Model, type Provider, type TranscriptContext,
} from "@earendil-works/pi-ai";
import { estimateTokens, type ToolDefinition } from "@earendil-works/pi-coding-agent";
import { assertActive, Cancelled } from "./errors.js";
import type { Initialize, TaskBudget } from "./protocol.js";

export type Workload = keyof Initialize["workload_budgets"];
export type ExecutionPriority = "interactive" | "background";
type ExecutionLimits = Initialize["execution"];
type Budget = Initialize["workload_budgets"][Workload];

interface Waiter {
  priority: ExecutionPriority;
  signal: AbortSignal | undefined;
  grant(release: () => void): void;
  reject(error: Error): void;
  cancel(): void;
}

function outputOptions<T extends { maxTokens?: number }>(options: T | undefined, maximum: number): T | undefined {
  return options ? { ...options, maxTokens: Math.min(options.maxTokens ?? maximum, maximum) } : undefined;
}

export class PriorityGate {
  private readonly pending: Waiter[] = [];
  private active = 0;
  private background = 0;

  constructor(private readonly capacity: number, private readonly reserve: number) {
    if (capacity < 1 || reserve < 0 || reserve >= capacity) throw new Error("Invalid execution gate");
  }

  acquire(priority: ExecutionPriority, signal?: AbortSignal): Promise<() => void> {
    assertActive(signal);
    return new Promise((grant, reject) => {
      const waiter: Waiter = {
        priority, signal, grant, reject,
        cancel: () => {
          const index = this.pending.indexOf(waiter);
          if (index >= 0) this.pending.splice(index, 1);
          reject(new Cancelled("Queued execution was cancelled"));
          this.drain();
        },
      };
      this.pending.push(waiter);
      signal?.addEventListener("abort", waiter.cancel, { once: true });
      this.drain();
    });
  }

  private drain(): void {
    while (this.active < this.capacity) {
      let index = this.pending.findIndex(waiter => waiter.priority === "interactive");
      if (index < 0 && this.background < this.capacity - this.reserve) {
        index = this.pending.findIndex(waiter => waiter.priority === "background");
      }
      if (index < 0) return;
      const waiter = this.pending.splice(index, 1)[0];
      if (!waiter) return;
      waiter.signal?.removeEventListener("abort", waiter.cancel);
      this.active++;
      if (waiter.priority === "background") this.background++;
      let released = false;
      waiter.grant(() => {
        if (released) return;
        released = true;
        this.active--;
        if (waiter.priority === "background") this.background--;
        this.drain();
      });
    }
  }

  snapshot() {
    return { active: this.active, queued: this.pending.length, background: this.background, capacity: this.capacity };
  }
}

export class ExecutionControl {
  private readonly context = new AsyncLocalStorage<{ workload: Workload; priority: ExecutionPriority }>();
  readonly limits: ExecutionLimits;
  private readonly models: PriorityGate;
  readonly tools: PriorityGate;
  private readonly budgets: Record<Workload, Budget>;

  constructor(config: Initialize) {
    this.limits = config.execution;
    if (this.limits.foreground_reserve >= this.limits.model_concurrency) throw new Error("Foreground reserve must leave background capacity");
    this.models = new PriorityGate(this.limits.model_concurrency, this.limits.foreground_reserve);
    this.tools = new PriorityGate(this.limits.tool_concurrency, this.limits.tool_concurrency > 1 ? 1 : 0);
    this.budgets = config.workload_budgets;
  }

  run<T>(workload: Workload, priority: ExecutionPriority, operation: () => T): T {
    return this.context.run({ workload, priority }, operation);
  }

  timeout(workload: Workload): number {
    const duration = this.budgets[workload].wall_clock;
    return duration.secs * 1000 + duration.nanos / 1_000_000;
  }

  taskBudget(request: TaskBudget | undefined, timeoutSeconds: number, turns: number): Required<TaskBudget> {
    const budget = {
      timeout_seconds: request?.timeout_seconds ?? Math.min(timeoutSeconds, this.limits.task_timeout_seconds),
      max_turns: request?.max_turns ?? Math.min(turns, this.limits.task_max_turns),
    };
    if (budget.timeout_seconds > this.limits.task_timeout_seconds || budget.max_turns > this.limits.task_max_turns) {
      throw new Error(`Task budget exceeds owner ceilings: ${this.limits.task_timeout_seconds} seconds and ${this.limits.task_max_turns} turns`);
    }
    return budget;
  }

  budget(): Budget {
    return this.budgets[this.context.getStore()?.workload ?? "chat"];
  }

  priority(): ExecutionPriority {
    return this.context.getStore()?.priority ?? "interactive";
  }

  limitModel<T extends Api>(model: Model<T>, force = false): Model<T> {
    if (!force && !this.context.getStore()) return model;
    const budget = this.budget();
    const maxTokens = Math.min(model.maxTokens, budget.max_output_tokens);
    return { ...model, maxTokens, contextWindow: Math.min(model.contextWindow, budget.max_prompt_tokens + maxTokens) };
  }

  provider(provider: Provider): Provider {
    return {
      ...provider,
      stream: (model, context, options) => this.stream(model, context, options?.signal,
        () => provider.stream(this.limitModel(model, true), context, outputOptions(options, this.budget().max_output_tokens))),
      streamSimple: (model, context, options) => this.stream(model, context, options?.signal,
        () => provider.streamSimple(this.limitModel(model, true), context, {
          ...options, maxTokens: Math.min(options?.maxTokens ?? model.maxTokens, this.budget().max_output_tokens),
        })),
    };
  }

  tool(tool: ToolDefinition): ToolDefinition {
    if (tool.name === "subagent" || tool.name.startsWith("background_task")) return tool;
    return {
      ...tool,
      execute: async (id, args, signal, update, context) => {
        const release = await this.tools.acquire(this.priority(), signal);
        try {
          assertActive(signal);
          return await tool.execute(id, args, signal, update, context);
        } finally { release(); }
      },
    };
  }

  private stream(
    model: Model<Api>, context: TranscriptContext, signal: AbortSignal | undefined,
    start: () => AssistantMessageEventStream,
  ): AssistantMessageEventStream {
    const output = createAssistantMessageEventStream();
    void (async () => {
      let release: (() => void) | undefined;
      try {
        release = await this.models.acquire(this.priority(), signal);
        assertActive(signal);
        const estimated = context.messages.reduce((sum, message) => sum + estimateTokens(message), 0);
        if (estimated > this.budget().max_prompt_tokens) {
          throw new Error(`context_length_exceeded: estimated input ${estimated} exceeds the configured ${this.budget().max_prompt_tokens} token budget; compact context or narrow the task`);
        }
        const source = start();
        for await (const event of source) output.push(event);
        output.end(await source.result());
      } catch (error) {
        const message: AssistantMessage = {
          role: "assistant", content: [], api: model.api, provider: model.provider, model: model.id,
          stopReason: signal?.aborted ? "aborted" : "error",
          errorMessage: error instanceof Error ? error.message : "Model admission failed",
          timestamp: Date.now(), usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0,
            cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } },
        };
        output.push({ type: "error", reason: signal?.aborted ? "aborted" : "error", error: message });
        output.end(message);
      } finally { release?.(); }
    })();
    return output;
  }

  snapshot() {
    return { models: this.models.snapshot(), tools: this.tools.snapshot() };
  }
}
