import { Type } from "typebox";
import { defineTool, type ToolDefinition } from "@earendil-works/pi-coding-agent";
import { object, taskBudgetSchema } from "./protocol.ts";
import { sourceKey, type JobControl } from "./background.ts";

export interface DeferredOwnerInput {
  id: string;
  prompt: string;
}

export function ownerInputTool(
  control: JobControl, defer: (input: DeferredOwnerInput) => void,
): ToolDefinition {
  return defineTool({
    name: "bobe_owner_input",
    label: "Ask an important question",
    description: "Use only when a meaningful owner decision or missing personal fact is necessary. Routine errors, retries and tool permissions never need this. An ask saves the exact action, question and deferred continuation, then ends this run without holding a worker. The person can answer in chat, voice or the app; Rust resumes the continuation once. Approval choices must have values approve and decline, with clear local-language labels; omit choices to use Yes/No. Information can use choices or a short free-form answer. Present repeats a saved question in this channel without changing it. Never carry out the deferred action before its answer.",
    parameters: Type.Union([
      Type.Object({
        operation: Type.Literal("ask"),
        kind: Type.Enum(["approval","information"]),
        question: Type.String({ minLength:1,maxLength:2000 }),
        action: Type.String({ minLength:1,maxLength:2000 }),
        task: Type.String({ minLength:1,maxLength:32768 }),
        context: Type.Optional(Type.String({ maxLength:8000 })),
        choices: Type.Optional(Type.Array(Type.Object({
          value:Type.String({ minLength:1,maxLength:128 }),label:Type.String({ minLength:1,maxLength:128 }),
        },{additionalProperties:false}),{maxItems:8})),
        expires_seconds: Type.Optional(Type.Integer({ minimum:60,maximum:604800 })),
        budget:Type.Optional(taskBudgetSchema),
      },{additionalProperties:false}),
      Type.Object({operation:Type.Literal("list")},{additionalProperties:false}),
      Type.Object({
        operation:Type.Enum(["get","present","cancel"]),
        request_id:Type.String({format:"uuid"}),
      },{additionalProperties:false}),
    ]),
    execute:async (callId,args,signal) => {
      const {operation,...parameters}=args;
      const value=await control.call(operation,object(parameters),sourceKey(control,callId),signal);
      const stop=operation==="ask" || operation==="present";
      if (stop) {
        const input=object(value);
        if (typeof input.id!=="string" || typeof input.prompt!=="string") throw new Error("Invalid durable owner question");
        defer({id:input.id,prompt:input.prompt});
      }
      return {content:[{type:"text",text:JSON.stringify(value)}],details:value,terminate:stop};
    },
  });
}
