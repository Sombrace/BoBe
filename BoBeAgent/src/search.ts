import { access, stat } from "node:fs/promises";
import { constants } from "node:fs";
import { homedir } from "node:os";
import { basename, delimiter, dirname, join, relative, resolve } from "node:path";
import { StringDecoder } from "node:string_decoder";
import { fileURLToPath } from "node:url";
import {
  createFindToolDefinition, createGrepToolDefinition, DEFAULT_MAX_BYTES, formatSize,
  truncateHead, truncateLine, type GrepToolDetails,
} from "@earendil-works/pi-coding-agent";
import { asError, assertActive, onAbort } from "./errors.ts";
import { isRecord } from "./protocol.ts";
import { environment, type ProcessClient } from "./process.ts";

function absolutePath(value: string, cwd: string): string {
  let path = value.replace(/[\u00a0\u2000-\u200a\u202f\u205f\u3000]/g, " ").replace(/^@/, "");
  if (path === "~") path = homedir();
  else if (path.startsWith("~/")) path = join(homedir(), path.slice(2));
  if (path.startsWith("file://")) path = fileURLToPath(path);
  return resolve(cwd, path);
}

async function exists(path: string): Promise<boolean> {
  return await stat(path, { throwIfNoEntry: false }) !== undefined;
}

async function executable(program:"rg"|"fd",agentDir:string):Promise<string> {
  const candidates=[
    join(agentDir,"bin",program),
    ...(process.env.PATH ?? "").split(delimiter).filter(Boolean).flatMap(directory=>
      (program==="fd" ? ["fd","fdfind"] : ["rg"]).map(name=>join(directory,name))),
  ];
  for (const candidate of candidates) {
    if (!(await stat(candidate, { throwIfNoEntry: false }))?.isFile()) continue;
    await access(candidate, constants.X_OK);
    return candidate;
  }
  throw new Error(`${program} is not installed in the authority runtime`);
}

async function runSearch(
  owner: ProcessClient, requestId: number, program: "rg" | "fd", args: string[], cwd: string,agentDir:string,
  signal: AbortSignal | undefined, line: (value: string) => boolean,
): Promise<void> {
  assertActive(signal);
  let child: Awaited<ReturnType<ProcessClient["start"]>> | undefined;
  let stopRequested = false;
  let stopping: Promise<void> | undefined;
  let failure: Error | undefined;
  let pending = "";
  let stderr = "";
  let bytes = 0;
  const decoder = new StringDecoder("utf8");
  const stop = (): void => {
    stopRequested = true;
    if (child) {
      stopping ??= child.stop();
      void stopping.catch(() => {});
    }
  };
  try {
    using _abort = onAbort(signal, stop);
    child = await owner.start({ program:await executable(program,agentDir), args, cwd, env: environment(), purpose: "shell", request_id: requestId }, (stream, data) => {
      if (stream === "stderr") { stderr = (stderr + data.toString("utf8")).slice(-65536); return; }
      if (stopRequested) return;
      try {
        bytes += data.length;
        if (bytes > 16 * 1024 * 1024) throw new Error("Search output exceeded 16 MiB; narrow the search");
        pending += decoder.write(data);
        let newline = pending.indexOf("\n");
        while (newline >= 0) {
          const current = pending.slice(0, newline);
          pending = pending.slice(newline + 1);
          if (line(current)) { stop(); break; }
          newline = pending.indexOf("\n");
        }
      } catch (error) {
        failure = asError(error, "Search output could not be read");
        stop();
      }
    });
    if (stopRequested || signal?.aborted) stop();
    const code = await child.completion;
    await stopping;
    assertActive(signal);
    if (failure) throw failure;
    if (!stopRequested && code !== 0 && !(program === "rg" && code === 1)) {
      throw new Error(stderr.trim() || `${program} exited with code ${code}`);
    }
    if (!stopRequested) {
      pending += decoder.end();
      if (pending) line(pending);
    }
  } finally {
    await child?.stop();
    await stopping;
  }
}

export function ownedFind(cwd: string, owner: ProcessClient, requestId: () => number,agentDir:string): ReturnType<typeof createFindToolDefinition> {
  const definition = createFindToolDefinition(cwd);
  return {
    ...definition,
    execute: async (id, args, signal, update, context) => {
      let searching: Promise<string[]> | undefined;
      const tool = createFindToolDefinition(cwd, { operations: {
        exists,
        glob: (pattern, searchPath, options) => {
          searching = (async () => {
            const limit = Math.max(1, options.limit);
            const flags = ["--glob", "--color=never", "--hidden", "--max-results", String(limit)];
            let insideGit = false;
            for (let directory = searchPath; ;) {
              if (await exists(join(directory, ".git"))) { insideGit = true; break; }
              const parent = dirname(directory);
              if (parent === directory) break;
              directory = parent;
            }
            if (!insideGit) flags.push("--no-require-git");
            if (pattern.includes("/")) {
              flags.push("--full-path");
              if (!pattern.startsWith("/") && !pattern.startsWith("**/") && pattern !== "**") pattern = `**/${pattern}`;
            }
            flags.push("--", pattern, searchPath);
            const results: string[] = [];
            await runSearch(owner, requestId(), "fd", flags, cwd, agentDir,signal, line => {
              results.push(line.replace(/\r$/, ""));
              return results.length >= limit;
            });
            return results;
          })();
          return searching;
        },
      } });
      try { return await tool.execute(id, args, signal, update, context); }
      finally { await searching; }
    },
  };
}

function rgText(value: unknown): string {
  if (!isRecord(value)) return "";
  if (typeof value.text === "string") return value.text;
  if (typeof value.bytes === "string") return Buffer.from(value.bytes, "base64").toString("utf8");
  return "";
}

export function ownedGrep(cwd: string, owner: ProcessClient, requestId: () => number,agentDir:string): ReturnType<typeof createGrepToolDefinition> {
  const definition = createGrepToolDefinition(cwd);
  return {
    ...definition,
    execute: async (_id, params, signal, _update, context) => {
      const searchPath = absolutePath(params.path || ".", context?.cwd || cwd);
      const directory = (await stat(searchPath)).isDirectory();
      const limit = Math.max(1, Math.floor(params.limit ?? 100));
      const contextLines = params.context && params.context > 0 ? Math.min(Math.floor(params.context), Number.MAX_SAFE_INTEGER) : 0;
      const flags = ["--no-config", "--json", "--line-number", "--color=never", "--hidden"];
      if (params.ignoreCase) flags.push("--ignore-case");
      if (params.literal) flags.push("--fixed-strings");
      if (params.glob) flags.push("--glob", params.glob.replace(/^(!?)(?:\.\/)+/, "$1"));
      if (contextLines) flags.push("--context", String(contextLines));
      flags.push("--", params.pattern, directory ? "." : searchPath);
      const lines: string[] = [];
      let bytes = 0;
      let lineCount = 0;
      let matches = 0;
      let lastFile: string | undefined;
      let limitHit = false;
      let trailingEnd = 0;
      let limitReached = false;
      let linesTruncated = false;
      const push = (file: string, number: number, text: string, match: boolean): void => {
        lastFile = file;
        const path = directory ? relative(searchPath, resolve(searchPath, file)) : "";
        const label = path && !path.startsWith("..") ? path.replaceAll("\\", "/") : basename(file);
        const line = truncateLine(text.replace(/\r?\n?$/, ""));
        linesTruncated ||= line.wasTruncated;
        const formatted = match ? `${label}:${number}: ${line.text}` : `${label}-${number}- ${line.text}`;
        if (bytes - 1 <= DEFAULT_MAX_BYTES) lines.push(formatted);
        bytes += Buffer.byteLength(formatted, "utf8") + 1;
        lineCount++;
      };
      await runSearch(owner, requestId(), "rg", flags, directory ? searchPath : cwd, agentDir,signal, raw => {
        if (!raw.trim()) return false;
        const event: unknown = JSON.parse(raw);
        if (!isRecord(event) || (event.type !== "match" && event.type !== "context") || !isRecord(event.data)) return false;
        const match = event.type === "match";
        const file = rgText(event.data.path);
        const number = event.data.line_number;
        if (!file || typeof number !== "number") return false;
        const text = rgText(event.data.lines);
        if (!limitHit) {
          push(file, number, text, match);
          if (match && ++matches >= limit) {
            limitHit = true;
            trailingEnd = number + contextLines;
          }
          return false;
        }
        const inWindow = file === lastFile && number <= trailingEnd;
        if (inWindow) push(file, number, text, false);
        if (match) limitReached = true;
        return limitReached && !(inWindow && number < trailingEnd);
      });
      if (!matches) return { content: [{ type: "text", text: "No matches found" }], details: undefined };
      const truncation = truncateHead(lines.join("\n"), { maxLines: Number.MAX_SAFE_INTEGER });
      truncation.totalLines = lineCount;
      truncation.totalBytes = bytes - 1;
      let output = truncation.content;
      const details: GrepToolDetails = {};
      const notices: string[] = [];
      if (limitReached) {
        notices.push(`${limit} matches limit reached. Use limit=${limit * 2} for more, or refine pattern`);
        details.matchLimitReached = limit;
      }
      if (truncation.truncated) {
        notices.push(`${formatSize(DEFAULT_MAX_BYTES)} limit reached`);
        details.truncation = truncation;
      }
      if (linesTruncated) {
        notices.push("Some lines truncated to 500 chars. Use read tool to see full lines");
        details.linesTruncated = true;
      }
      if (notices.length) output += `\n\n[${notices.join(". ")}]`;
      return { content: [{ type: "text", text: output }], details: Object.keys(details).length ? details : undefined };
    },
  };
}
