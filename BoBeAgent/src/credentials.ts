import { createHash, randomUUID } from "node:crypto";
import { dirname, join } from "node:path";
import { unlink } from "node:fs/promises";
import type { AuthOperationOptions, Credential, CredentialInfo, CredentialStore, OAuthCredential } from "@earendil-works/pi-ai";
import { Serial, assertActive, type Redactor } from "./errors.js";
import { atomicPrivateWrite, isMissing, privateRead, syncFile } from "./files.js";
import { isJson, isRecord } from "./protocol.js";
import { AGENT_LAYOUT } from "./layout.js";

interface Scope {
  nativeProvider: string;
  apiKey?: string;
  oauth: boolean;
  serial: Serial;
}

function credential(value: unknown): OAuthCredential {
  if (!isRecord(value) || !isJson(value) || value.type !== "oauth" ||
    typeof value.access !== "string" || !value.access ||
    typeof value.refresh !== "string" || !value.refresh ||
    typeof value.expires !== "number" || !Number.isFinite(value.expires)) {
    throw new Error("Invalid stored OAuth credential");
  }
  return { ...value, type: "oauth", access: value.access, refresh: value.refresh, expires: value.expires };
}

export class AuthorityCredentials implements CredentialStore {
  private readonly scopes = new Map<string, Scope>();

  constructor(private readonly agentDir: string, private readonly redactor: Redactor) {}

  register(provider: string, nativeProvider: string, oauth: boolean, apiKey?: string): void {
    if (this.scopes.has(provider)) throw new Error("Duplicate credential scope");
    this.redactor.add(apiKey);
    this.scopes.set(provider, { nativeProvider, oauth, serial: new Serial(), ...(apiKey ? { apiKey } : {}) });
  }

  private path(provider: string): string {
    const key = createHash("sha256").update(provider).digest("hex");
    return join(this.agentDir, AGENT_LAYOUT.credentials, `${key}.json`);
  }

  async read(provider: string, options?: AuthOperationOptions): Promise<Credential | undefined> {
    assertActive(options?.signal);
    const scope = this.scopes.get(provider);
    if (!scope) return undefined;
    if (!scope.oauth) return scope.apiKey ? { type: "api_key", key: scope.apiKey } : undefined;
    const stored = await privateRead(this.path(provider));
    assertActive(options?.signal);
    if (stored === undefined) return undefined;
    let parsed: unknown;
    try {
      parsed = JSON.parse(stored);
    } catch (error) {
      throw new Error("Invalid authority-scoped OAuth file", { cause: error });
    }
    if (!isRecord(parsed) || parsed.version !== 1 || parsed.provider !== scope.nativeProvider) {
      throw new Error("Stored credential does not match its owner profile");
    }
    const result = credential(parsed.credential);
    this.redactor.add(result.access);
    this.redactor.add(result.refresh);
    return result;
  }

  async list(options?: AuthOperationOptions): Promise<readonly CredentialInfo[]> {
    const results: CredentialInfo[] = [];
    for (const providerId of this.scopes.keys()) {
      const value = await this.read(providerId, options);
      if (value) results.push({ providerId, type: value.type });
    }
    return results;
  }

  async modify(
    provider: string,
    change: (current: Credential | undefined) => Promise<Credential | undefined>,
    options?: AuthOperationOptions,
  ): Promise<Credential | undefined> {
    const scope = this.scopes.get(provider);
    if (!scope) throw new Error("Unknown credential scope");
    return scope.serial.run(async () => {
      assertActive(options?.signal);
      const current = await this.read(provider, options);
      const next = await change(current);
      if (!next) return current;
      if (!scope.oauth || next.type !== "oauth") throw new Error("Only native OAuth may persist worker credentials");
      const checked = credential(next);
      this.redactor.add(checked.access);
      this.redactor.add(checked.refresh);
      assertActive(options?.signal);
      await atomicPrivateWrite(this.path(provider), `${JSON.stringify({
        version: 1,
        provider: scope.nativeProvider,
        credential: checked,
      })}\n`);
      return checked;
    });
  }

  async delete(provider: string, options?: AuthOperationOptions): Promise<void> {
    const scope = this.scopes.get(provider);
    if (!scope) throw new Error("Unknown credential scope");
    await scope.serial.run(async () => {
      assertActive(options?.signal);
      if (!scope.oauth) throw new Error("API keys are owned by the daemon");
      const path = this.path(provider);
      try {
        await unlink(path);
      } catch (error) {
        if (!isMissing(error)) throw error;
      }
      try {
        await syncFile(dirname(path));
      } catch (error) {
        if (!isMissing(error)) throw error;
      }
    });
  }
}

export async function deviceId(agentDir: string): Promise<string> {
  const path = join(agentDir, AGENT_LAYOUT.device_id);
  const existing = await privateRead(path);
  if (existing !== undefined) {
    if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\n?$/.test(existing)) {
      throw new Error("Invalid worker device identity");
    }
    return existing.trim();
  }
  const id = randomUUID();
  await atomicPrivateWrite(path, `${id}\n`);
  return id;
}
