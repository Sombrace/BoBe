import assert from "node:assert/strict";
import test from "node:test";
import { TurnEvents } from "./events.ts";
import type { Delta } from "./protocol.ts";

test("tool calls that pi skips after a stop report no start, activity or completion", () => {
  const deltas: Delta[] = [];
  const activity: string[] = [];
  const events = new TurnEvents(delta => deltas.push(delta), name => activity.push(name));
  events.event({ type: "tool_execution_start", toolCallId: "running", toolName: "bash", args: {} });
  events.stop();
  events.event({ type: "tool_execution_start", toolCallId: "skipped", toolName: "web_search", args: {} });
  events.event({ type: "tool_execution_end", toolCallId: "skipped", toolName: "web_search", result: {}, isError: true });
  events.event({ type: "tool_execution_end", toolCallId: "running", toolName: "bash", result: {}, isError: true });
  assert.deepEqual(activity, ["bash"]);
  assert.deepEqual(deltas.map(delta => [delta.type, "name" in delta ? delta.name : undefined]), [
    ["tool_start", "bash"], ["tool_complete", "bash"],
  ]);
  assert.equal(events.completedTools, 1);
});
