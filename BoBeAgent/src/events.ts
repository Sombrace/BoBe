import { randomUUID } from "node:crypto";
import type { AgentSessionEvent } from "@earendil-works/pi-coding-agent";
import type { AssistantMessage, Usage } from "@earendil-works/pi-ai";
import { type Delta } from "./protocol.ts";

interface ReplyMessage {
  id: string;
  chunks: string[];
  message: AssistantMessage | undefined;
}

export function addUsage(total: Usage | undefined, next: Usage): Usage {
  const sum = (key: "input" | "output" | "cacheRead" | "cacheWrite"): number => (total?.[key] ?? 0) + next[key];
  const cost = (key: keyof Usage["cost"]): number => (total?.cost[key] ?? 0) + next.cost[key];
  return {
    input: sum("input"), output: sum("output"), cacheRead: sum("cacheRead"), cacheWrite: sum("cacheWrite"),
    totalTokens: (total?.totalTokens ?? 0) + next.totalTokens,
    cost: {
      input: cost("input"), output: cost("output"), cacheRead: cost("cacheRead"),
      cacheWrite: cost("cacheWrite"), total: cost("total"),
    },
  };
}

export class TurnEvents {
  private current: ReplyMessage | undefined;
  private final: ReplyMessage | undefined;
  private readonly openTools = new Map<string, { id: string; name: string }>();
  private failure: string | undefined;
  last: AssistantMessage | undefined;
  usage: Usage | undefined;
  textDeltas = 0;
  completedTools = 0;
  failedTools = 0;
  turns = 0;

  private readonly send: ((delta: Delta) => void) | undefined;
  private readonly activity: ((name: string) => void) | undefined;

  constructor(send: ((delta: Delta) => void) | undefined, activity: ((name: string) => void) | undefined) {
    this.send = send;
    this.activity = activity;
  }

  event(event: AgentSessionEvent): void {
    if (event.type === "message_start" && event.message.role === "assistant") {
      this.current = { id: randomUUID(), chunks: [], message: undefined };
    } else if (event.type === "message_update" && event.message.role === "assistant" &&
      event.assistantMessageEvent.type === "text_delta") {
      this.textDeltas++;
      this.current?.chunks.push(event.assistantMessageEvent.delta);
      if (this.current && !this.failure) {
        this.send?.({
          type: "message_delta", message_id: this.current.id, text: event.assistantMessageEvent.delta,
        });
      }
    } else if (event.type === "message_end" && event.message.role === "toolResult" && event.message.usage) {
      this.usage = addUsage(this.usage, event.message.usage);
    } else if (event.type === "message_end" && event.message.role === "assistant") {
      this.last = event.message;
      this.usage = addUsage(this.usage, event.message.usage);
      if (this.current) {
        this.current.message = event.message;
        if (event.message.stopReason === "stop") this.final = this.current;
        if (event.message.stopReason === "toolUse" && !this.failure &&
          event.message.content.some(block => block.type === "text" && block.text.length > 0)) {
          this.completeMessage(this.current);
        }
      }
      this.current = undefined;
    } else if (event.type === "tool_execution_start") {
      const tool = { id: randomUUID(), name: event.toolName };
      this.openTools.set(event.toolCallId, tool);
      this.activity?.(event.toolName);
      this.send?.({ type: "tool_start", id: tool.id, name: tool.name });
    } else if (event.type === "tool_execution_end") {
      const tool = this.openTools.get(event.toolCallId);
      this.openTools.delete(event.toolCallId);
      this.completedTools++;
      if (event.isError) this.failedTools++;
      this.send?.({
        type: "tool_complete", id: tool?.id ?? randomUUID(), name: event.toolName, success: !event.isError,
      });
    } else if (event.type === "compaction_end" && event.errorMessage) {
      this.failure = `Session compaction failed: ${event.errorMessage}`;
    } else if (event.type === "turn_end") {
      this.turns++;
    }
  }

  assertSuccess(allowToolTermination = false): void {
    if (this.failure) throw new Error(this.failure);
    if (!this.last) throw new Error("pi returned no assistant message");
    if (this.last.stopReason === "error" || this.last.stopReason === "aborted") {
      throw new Error(this.last.errorMessage ?? `Provider ${this.last.stopReason}`);
    }
    if (this.last.stopReason !== "stop" && !(allowToolTermination && this.last.stopReason === "toolUse")) {
      throw new Error(`Provider did not complete its response (${this.last.stopReason})`);
    }
    if (this.openTools.size) throw new Error("pi settled with unfinished tool calls");
  }

  text(): string {
    return this.last?.content.filter(block => block.type === "text").map(block => block.text).join("") ?? "";
  }

  commit(): void {
    this.assertSuccess();
    const reply = this.final;
    if (!reply?.message) throw new Error("pi returned no completed parent reply");
    this.completeMessage(reply);
    if (this.failure) throw new Error(this.failure);
    this.send?.({ type: "done" });
  }

  commitOwnerQuestion(prompt:string):void {
    this.assertSuccess(true);
    const id=randomUUID();
    this.send?.({type:"message_delta",message_id:id,text:prompt});
    this.send?.({type:"message_complete",message_id:id,content:prompt,output_tokens:0});
    this.send?.({type:"done"});
  }

  private completeMessage(reply: ReplyMessage): void {
    if (!reply.message) return;
    const content = reply.message.content.filter(block => block.type === "text").map(block => block.text).join("");
    const streamed = reply.chunks.join("");
    if (!content.startsWith(streamed)) {
      this.failure = "Provider changed text that was already streamed";
      return;
    }
    const remaining = content.slice(streamed.length);
    if (remaining) {
      this.send?.({ type: "message_delta", message_id: reply.id, text: remaining });
    }
    this.send?.({
      type: "message_complete", message_id: reply.id, content,
      output_tokens: reply.message.usage.output,
    });
  }

  endFailedTools(): void {
    for (const { id, name } of this.openTools.values()) this.send?.({ type: "tool_complete", id, name, success: false });
    this.openTools.clear();
  }
}
