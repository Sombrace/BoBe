import { createInterface } from "node:readline";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { isRecord } from "./protocol.ts";

const pidFile = process.env.BOBE_TEST_PID_FILE;
if (pidFile) writeFileSync(pidFile, String(process.pid), { mode: 0o600 });
const startDirectory = process.env.BOBE_TEST_START_DIR;
if (startDirectory) {
  let ordinal = 1;
  for (;;) {
    try {
      writeFileSync(join(startDirectory, String(ordinal)), "", { flag: "wx", mode: 0o600 });
      break;
    } catch (error) {
      if (!isRecord(error) || error.code !== "EEXIST") throw error;
      ordinal++;
    }
  }
  if (ordinal > Number(process.env.BOBE_TEST_START_LIMIT)) process.exit(1);
}
const waits = new Map<string | number, ReturnType<typeof setTimeout>>();
const input = createInterface({ input: process.stdin });
input.on("line", line => {
  const value: unknown = JSON.parse(line);
  if (!isRecord(value) || typeof value.method !== "string") return;
  if (value.method === "notifications/cancelled" && isRecord(value.params)) {
    const id = value.params.requestId;
    if (typeof id === "string" || typeof id === "number") {
      const timer = waits.get(id);
      if (timer) {
        clearTimeout(timer);
        waits.delete(id);
        process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id, error: { code: -32800, message: "Canceled" } })}\n`);
      }
    }
    return;
  }
  if (typeof value.id !== "number" && typeof value.id !== "string") return;
  const id = value.id;
  const reply = (result: unknown): void => {
    process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id, result })}\n`);
  };
  if (value.method === "initialize") {
    reply({ protocolVersion: "2025-06-18", capabilities: { tools: {} }, serverInfo: { name: "bobe-test", version: "1" } });
  } else if (value.method === "tools/list") {
    const paginated = process.env.BOBE_TEST_PAGINATION === "1";
    const secondPage = isRecord(value.params) && value.params.cursor === "next";
    const names = paginated ? secondPage ? ["wait"] : ["echo", "hidden"] : ["echo", "hidden", "wait"];
    reply({ tools: names.map(name => ({
      name, description: `Controlled ${name} tool`,
      inputSchema: { type: "object", properties: { text: { type: "string" } }, required: ["text"] },
    })), ...(paginated && !secondPage ? { nextCursor: "next" } : {}) });
  } else if (value.method === "tools/call" && isRecord(value.params)) {
    if (process.env.BOBE_TEST_EXIT_ON_CALL) process.exit(Number(process.env.BOBE_TEST_EXIT_ON_CALL));
    const args = value.params.arguments;
    const text = isRecord(args) && typeof args.text === "string" ? args.text : "";
    if (value.params.name === "wait") {
      if (process.env.BOBE_TEST_CALL_FILE) writeFileSync(process.env.BOBE_TEST_CALL_FILE, "", { mode: 0o600 });
      waits.set(id, setTimeout(() => {
        waits.delete(id);
        reply({ content: [{ type: "text", text }] });
      }, 30_000));
    } else {
      reply({ content: [{ type: "text", text }] });
    }
  } else if (value.method === "ping") {
    reply({});
  } else {
    process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", id, error: { code: -32601, message: "Method not found" } })}\n`);
  }
});
input.on("close", () => {
  for (const timer of waits.values()) clearTimeout(timer);
});
