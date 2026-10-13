import { isAbsolute, join, relative, resolve } from "node:path";
import { readdir, open } from "node:fs/promises";
import { SessionManager, CURRENT_SESSION_VERSION } from "@earendil-works/pi-coding-agent";
import { atomicPrivateWrite, privateDirectory, privateRead, syncFile } from "./files.js";
import { isJson, isRecord } from "./protocol.js";
import { AGENT_LAYOUT } from "./layout.js";

interface Pointer {
  version: 1;
  session_id: string;
  file: string;
}

function persistedJson(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error("Invalid JSON in canonical session state", { cause: error });
  }
}

function parsePointer(text: string): Pointer {
  const value = persistedJson(text);
  if (!isRecord(value) || value.version !== 1 || typeof value.session_id !== "string" ||
    typeof value.file !== "string" || !/^sessions\/[A-Za-z0-9_.-]+\.jsonl$/.test(value.file)) {
    throw new Error("Invalid canonical session pointer");
  }
  return { version: 1, session_id: value.session_id, file: value.file };
}

function nonempty(value: unknown): value is string {
  return typeof value === "string" && value.length > 0;
}

function finite(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function content(value: unknown, allowed: ReadonlySet<string>, allowString = true): boolean {
  if (typeof value === "string") return allowString;
  if (!Array.isArray(value)) return false;
  return value.every(block => {
    if (!isRecord(block) || typeof block.type !== "string" || !allowed.has(block.type)) return false;
    switch (block.type) {
      case "text": return typeof block.text === "string";
      case "image": return nonempty(block.mimeType) && typeof block.data === "string";
      case "thinking": return typeof block.thinking === "string";
      case "toolCall": return nonempty(block.id) && nonempty(block.name) && isRecord(block.arguments);
      default: return false;
    }
  });
}

const textBlocks = new Set(["text"]);
const inputBlocks = new Set(["text", "image"]);
const assistantBlocks = new Set(["text", "thinking", "toolCall"]);
const editableBlocks = new Set([...inputBlocks, ...assistantBlocks]);
const stopReasons = new Set(["stop", "length", "toolUse", "error", "aborted", "deferred"]);
const thinkingLevels = new Set(["off", "minimal", "low", "medium", "high", "xhigh", "max"]);

function usage(value: unknown): boolean {
  return isRecord(value) && ["input", "output", "cacheRead", "cacheWrite", "totalTokens"].every(key => finite(value[key])) &&
    isRecord(value.cost) && ["input", "output", "cacheRead", "cacheWrite", "total"].every(key =>
      isRecord(value.cost) && finite(value.cost[key]));
}

function message(value: unknown): boolean {
  if (!isRecord(value) || !finite(value.timestamp)) return false;
  switch (value.role) {
    case "system":
      return content(value.content, textBlocks) &&
        (value.sections === undefined || (isRecord(value.sections) &&
          Object.values(value.sections).every(section => section === null || typeof section === "string"))) &&
        (value.toolsAdded === undefined || (Array.isArray(value.toolsAdded) && value.toolsAdded.every(tool =>
          isRecord(tool) && nonempty(tool.name) && typeof tool.description === "string" && isRecord(tool.parameters)))) &&
        (value.toolsRemoved === undefined || Array.isArray(value.toolsRemoved));
    case "user":
      return content(value.content, inputBlocks);
    case "assistant":
      return content(value.content, assistantBlocks, false) &&
        nonempty(value.api) && nonempty(value.provider) && nonempty(value.model) &&
        typeof value.stopReason === "string" && stopReasons.has(value.stopReason) && usage(value.usage) &&
        (value.errorMessage === undefined || typeof value.errorMessage === "string");
    case "toolResult":
      return nonempty(value.toolCallId) && nonempty(value.toolName) &&
        typeof value.isError === "boolean" && content(value.content, inputBlocks, false);
    case "bashExecution":
      return typeof value.command === "string" && typeof value.output === "string" &&
        typeof value.cancelled === "boolean" && typeof value.truncated === "boolean";
    case "custom":
      return nonempty(value.customType) && typeof value.display === "boolean" && content(value.content, inputBlocks);
    default:
      return false;
  }
}

function entryPayload(entry: Record<string, unknown>, ids: ReadonlySet<string>): boolean {
  switch (entry.type) {
    case "message": return message(entry.message);
    case "thinking_level_change": return typeof entry.thinkingLevel === "string" && thinkingLevels.has(entry.thinkingLevel);
    case "model_change": return nonempty(entry.provider) && nonempty(entry.modelId);
    case "compaction":
      return typeof entry.summary === "string" && finite(entry.tokensBefore) &&
        (entry.firstKeptEntryId === null || (nonempty(entry.firstKeptEntryId) && ids.has(entry.firstKeptEntryId))) &&
        (entry.systemMessage === undefined || message(entry.systemMessage));
    case "branch_summary": return typeof entry.summary === "string" && nonempty(entry.fromId) && ids.has(entry.fromId);
    case "custom": return nonempty(entry.customType);
    case "custom_message":
      return nonempty(entry.customType) && typeof entry.display === "boolean" && content(entry.content, inputBlocks);
    case "context_edit":
      return nonempty(entry.targetId) && ids.has(entry.targetId) &&
        (entry.replacement === null || (isRecord(entry.replacement) && content(entry.replacement.content, editableBlocks)));
    case "label": return nonempty(entry.targetId) && ids.has(entry.targetId) &&
      (entry.label === undefined || typeof entry.label === "string");
    case "session_info": return entry.name === undefined || typeof entry.name === "string";
    case "usage": return nonempty(entry.kind) && nonempty(entry.provider) && nonempty(entry.model) && usage(entry.usage);
    default: return false;
  }
}

function validateTranscript(text: string, pointer: Pointer): void {
  if (!text.endsWith("\n")) throw new Error("Canonical transcript has an incomplete final entry");
  const lines = text.trimEnd().split("\n");
  const header = persistedJson(lines[0] ?? "");
  if (!isRecord(header) || header.type !== "session" || header.id !== pointer.session_id ||
    header.version !== CURRENT_SESSION_VERSION || typeof header.cwd !== "string" || !isAbsolute(header.cwd) ||
    typeof header.timestamp !== "string" || !Number.isFinite(Date.parse(header.timestamp))) {
    throw new Error("Canonical transcript header does not match its pointer or pi version");
  }
  const ids = new Set<string>();
  for (const line of lines.slice(1)) {
    const entry = persistedJson(line);
    if (!isRecord(entry) || !isJson(entry) || !nonempty(entry.id) || ids.has(entry.id) ||
      typeof entry.timestamp !== "string" || !Number.isFinite(Date.parse(entry.timestamp)) ||
      !(entry.parentId === null || (typeof entry.parentId === "string" && ids.has(entry.parentId))) ||
      !entryPayload(entry, ids)) {
      throw new Error("Canonical transcript contains an invalid entry");
    }
    ids.add(entry.id);
  }
}

export async function openCanonicalSession(agentDir: string, workspace: string): Promise<SessionManager> {
  await privateDirectory(agentDir);
  const sessionDir = join(agentDir, AGENT_LAYOUT.transcripts);
  await privateDirectory(sessionDir);
  const pointerPath = join(agentDir, AGENT_LAYOUT.session_pointer);
  const stored = await privateRead(pointerPath);
  if (stored !== undefined) {
    const pointer = parsePointer(stored);
    const path = resolve(agentDir, pointer.file);
    const transcript = await privateRead(path);
    if (transcript === undefined) throw new Error("Canonical session pointer targets a missing transcript");
    validateTranscript(transcript, pointer);
    const manager = SessionManager.open(path, sessionDir, workspace);
    if (manager.getSessionId() !== pointer.session_id) throw new Error("pi changed the canonical session identity");
    return manager;
  }
  if ((await readdir(sessionDir)).length > 0) {
    throw new Error("Canonical session pointer is missing while transcripts exist; owner recovery is required");
  }
  const manager = await persistNewSession(SessionManager.create(workspace, sessionDir));
  const file = manager.getSessionFile();
  if (!file) throw new Error("pi did not create a persistent session identity");
  const pointer: Pointer = {
    version: 1,
    session_id: manager.getSessionId(),
    file: relative(agentDir, file),
  };
  await atomicPrivateWrite(pointerPath, `${JSON.stringify(pointer)}\n`);
  return manager;
}

async function persistNewSession(manager: SessionManager): Promise<SessionManager> {
  const file = manager.getSessionFile();
  const header = manager.getHeader();
  if (!file || !header) throw new Error("pi did not create a persistent session identity");
  const handle = await open(file, "wx", 0o600);
  try {
    await handle.writeFile(`${JSON.stringify(header)}\n`, "utf8");
    await handle.sync();
  } finally {
    await handle.close();
  }
  await syncFile(manager.getSessionDir());
  return SessionManager.open(file, manager.getSessionDir(), manager.getCwd());
}

export async function createChildSession(
  agentDir: string, workspace: string, parent: SessionManager,
): Promise<SessionManager> {
  const parentFile = parent.getSessionFile();
  if (!parentFile) throw new Error("A delegated transcript requires its persistent parent session");
  const directory = join(agentDir, AGENT_LAYOUT.child_transcripts);
  await privateDirectory(directory);
  return persistNewSession(SessionManager.create(workspace, directory, { parentSession: parentFile }));
}

export async function openBackgroundSession(agentDir: string, workspace: string, jobId: string): Promise<SessionManager> {
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(jobId)) {
    throw new Error("Background session requires a valid job identity");
  }
  return openCanonicalSession(join(agentDir, "sessions", "jobs", jobId), workspace);
}

export const OWNER_CHANNEL = "00000000-0000-0000-0000-000000000000";

export async function openChannelSession(agentDir: string, workspace: string, channel: string): Promise<SessionManager> {
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(channel)) {
    throw new Error("Conversation channel requires a valid identity");
  }
  return openCanonicalSession(channel === OWNER_CHANNEL ? agentDir : join(agentDir, "sessions/channels", channel), workspace);
}

export async function flushSession(manager: SessionManager): Promise<void> {
  const file = manager.getSessionFile();
  if (file) await syncFile(file);
}
