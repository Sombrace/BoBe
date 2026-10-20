import {
  chmod, lstat, mkdir, open, readFile, realpath, rename, unlink,
} from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { isRecord } from "./protocol.ts";

export function isMissing(error: unknown): boolean {
  return isRecord(error) && error.code === "ENOENT";
}

export async function privateDirectory(path: string): Promise<void> {
  await mkdir(path, { recursive: true, mode: 0o700 });
  const stat = await lstat(path);
  if (!stat.isDirectory() || stat.isSymbolicLink()) throw new Error("Private state directory is not a directory");
  if (process.getuid && stat.uid !== process.getuid()) throw new Error("Private state has a different owner");
  await chmod(path, 0o700);
}

export async function syncFile(path: string): Promise<void> {
  await using handle = await open(path, "r");
  await handle.sync();
}

export async function privateRead(path: string): Promise<string | undefined> {
  try {
    const stat = await lstat(path);
    if (!stat.isFile() || stat.isSymbolicLink()) throw new Error("Private state is not a regular file");
    if (process.getuid && stat.uid !== process.getuid()) throw new Error("Private state has a different owner");
    await chmod(path, 0o600);
    return await readFile(path, "utf8");
  } catch (error) {
    if (isMissing(error)) return undefined;
    throw error;
  }
}

export async function atomicPrivateWrite(path: string, content: string): Promise<void> {
  await privateDirectory(dirname(path));
  const staged = `${path}.${randomUUID()}.pending`;
  {
    await using pending = new AsyncDisposableStack();
    const file = pending.use(await open(staged, "wx", 0o600));
    pending.defer(() => unlink(staged));
    await file.writeFile(content, "utf8");
    await file.sync();
    await file.close();
    await rename(staged, path);
    pending.move();
  }
  await syncFile(dirname(path));
}

export async function authorityPath(root: string, path: string): Promise<string> {
  if (!isAbsolute(root) || !isAbsolute(path)) throw new Error("Authority paths must be absolute");
  const canonicalRoot = await realpath(root);
  const canonical = await realpath(path);
  const suffix = relative(canonicalRoot, canonical);
  if (suffix === ".." || suffix.startsWith("../") || isAbsolute(suffix)) {
    throw new Error("Resource path is outside the owner authority");
  }
  if (resolve(path) !== canonical) throw new Error("Authority resources must not use symbolic links");
  return canonical;
}
