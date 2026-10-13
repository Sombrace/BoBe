import { randomUUID } from "node:crypto";
import type { AgentSessionEvent } from "@earendil-works/pi-coding-agent";
import type { AssistantMessage } from "@earendil-works/pi-ai";
import { type Delta } from "./protocol.js";

interface ReplyMessage {
  id: string;
  chunks: string[];
  message: AssistantMessage | undefined;
}

export class TurnEvents {
  private current: ReplyMessage | undefined;
  private final: ReplyMessage | undefined;
  private readonly openTools = new Map<string, { id: string; name: string }>();
  private failure: string | undefined;
  private visibleOutput = false;
  last: AssistantMessage | undefined;
  textDeltas = 0;
  completedTools = 0;
  failedTools = 0;
  turns = 0;

  constructor(
    private readonly send: ((delta: Delta) => void) | undefined,
    private readonly activity: ((name: string) => void) | undefined,
  ) {}

  event(event: AgentSessionEvent): void {
    if (event.type === "message_start" && event.message.role === "assistant") {
      this.current = { id: randomUUID(), chunks: [], message: undefined };
    } else if (event.type === "message_update" && event.message.role === "assistant" &&
      event.assistantMessageEvent.type === "text_delta") {
      this.textDeltas++;
      this.current?.chunks.push(event.assistantMessageEvent.delta);
      if (this.current && !this.failure) {
        this.visibleOutput = true;
        this.send?.({
          type: "message_delta", message_id: this.current.id, text: event.assistantMessageEvent.delta,
        });
      }
    } else if (event.type === "message_end" && event.message.role === "assistant") {
      this.last = event.message;
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
    } else if (event.type === "auto_retry_start") {
      this.final = undefined;
      if (this.visibleOutput) this.failure = "Provider attempted a retry after visible reply text";
    } else if (event.type === "auto_retry_end" && !event.success) {
      this.failure = event.finalError ?? "Provider retries exhausted";
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
      this.visibleOutput = true;
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
