import { access, readFile, stat } from "node:fs/promises";
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

export function ownedGrep(cwd: string, owner: ProcessClient, requestId: () => number,agentDir:string): ReturnType<typeof createGrepToolDefinition> {
  const definition = createGrepToolDefinition(cwd);
  return {
    ...definition,
    execute: async (_id, params, signal, _update, context) => {
      const searchPath = absolutePath(params.path || ".", context?.cwd || cwd);
      const directory = (await stat(searchPath)).isDirectory();
      const limit = Math.max(1, params.limit ?? 100);
      const contextLines = Math.max(0, params.context ?? 0);
      const flags = ["--json", "--line-number", "--color=never", "--hidden"];
      if (params.ignoreCase) flags.push("--ignore-case");
      if (params.literal) flags.push("--fixed-strings");
      if (params.glob) flags.push("--glob", params.glob);
      flags.push("--", params.pattern, searchPath);
      const matches: { file: string; number: number; text: string }[] = [];
      await runSearch(owner, requestId(), "rg", flags, cwd, agentDir,signal, line => {
        if (!line.trim()) return false;
        const event: unknown = JSON.parse(line);
        if (!isRecord(event) || event.type !== "match") return false;
        if (!isRecord(event.data) || !isRecord(event.data.path) || !isRecord(event.data.lines)
          || typeof event.data.line_number !== "number" || typeof event.data.path.text !== "string"
          || typeof event.data.lines.text !== "string") throw new Error("Search result cannot be represented as UTF-8 text");
        matches.push({ file: event.data.path.text, number: event.data.line_number, text: event.data.lines.text });
        return matches.length >= limit;
      });
      if (!matches.length) return { content: [{ type: "text", text: "No matches found" }], details: undefined };
      let linesTruncated = false;
      const lines: string[] = [];
      const files = new Map<string, Promise<string[]>>();
      for (const match of matches) {
        assertActive(signal);
        const path = directory ? relative(searchPath, match.file) : "";
        const label = path && !path.startsWith("..") ? path.replaceAll("\\", "/") : basename(match.file);
        if (!contextLines) {
          const line = truncateLine(match.text.replace(/\r\n/g, "\n").replace(/\r/g, "").replace(/\n$/, ""));
          linesTruncated ||= line.wasTruncated;
          lines.push(`${label}:${match.number}: ${line.text}`);
        } else {
          const content = await files.getOrInsertComputed(match.file, async file =>
            (await readFile(file, "utf8")).replace(/\r\n/g, "\n").replace(/\r/g, "\n").split("\n"));
          for (let number = Math.max(1, match.number - contextLines); number <= Math.min(content.length, match.number + contextLines); number++) {
            const line = truncateLine(content[number - 1] ?? "");
            linesTruncated ||= line.wasTruncated;
            lines.push(number === match.number ? `${label}:${number}: ${line.text}` : `${label}-${number}- ${line.text}`);
          }
        }
      }
      const truncation = truncateHead(lines.join("\n"), { maxLines: Number.MAX_SAFE_INTEGER });
      let output = truncation.content;
      const details: GrepToolDetails = {};
      const notices: string[] = [];
      if (matches.length >= limit) {
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
