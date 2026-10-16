import assert from "node:assert/strict";
import test from "node:test";
import fsPromises, { readdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import type { Mode, PathLike } from "node:fs";
import { syncBuiltinESMExports } from "node:module";
import { join } from "node:path";
import type { OAuthCredential } from "@earendil-works/pi-ai";
import { ModelProfiles } from "./models.js";
import { AuthorityCredentials } from "./credentials.js";
import { Redactor } from "./errors.js";
import { configuration, controlledProvider, deferred, fixture } from "./test-support.js";
import { type AuthPhase, isRecord } from "./protocol.js";
import { openCanonicalSession } from "./persistence.js";

const credential: OAuthCredential = {
  type: "oauth", access: "fixture-oauth-access", refresh: "fixture-oauth-refresh",
  expires: Date.now() + 86_400_000, login: "fixture-account",
};

test("native model runtime OAuth uses callback input and persists only inside the owner authority", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  config.profiles.test = { kind: "open_ai", auth_method: "oauth" };
  const phases: AuthPhase[] = [];
  const awaitingSelect = deferred<void>();
  const awaitingSecret = deferred<void>();
  const profiles = await ModelProfiles.create(config, join(paths.root, "agent"), (_profile, phase) => {
    phases.push(phase);
    if (phase.phase === "awaiting_input" && !phase.password) awaitingSelect.resolve();
    if (phase.phase === "awaiting_input" && phase.password) awaitingSecret.resolve();
  }, new Redactor());
  t.after(() => profiles.close());
  profiles.runtime.registerNativeProvider({
    ...controlledProvider(() => ({ text: "unused" })),
    auth: {
      oauth: {
        name: "Controlled OAuth", isSubscription: true,
        login: async interaction => {
          interaction.notify({ type: "device_code", verificationUri: "https://example.invalid/login", userCode: "DEVICE" });
          interaction.notify({ type: "progress", message: "Waiting" });
          const method = await interaction.prompt({
            type: "select", message: "Choose a method", options: [{ id: "manual", label: "Manual" }],
          });
          assert.equal(method, "manual");
          assert.equal(await interaction.prompt({ type: "secret", message: "Paste code" }), "fixture-code");
          return { ...credential };
        },
        refresh: async () => ({ ...credential }),
        toAuth: async value => ({ apiKey: value.access }),
      },
    },
  });
  const login = profiles.login("test");
  await awaitingSelect.promise;
  assert.throws(() => profiles.submit("test", "invalid"), /offered/);
  const status = await profiles.status("test");
  assert.ok(isRecord(status) && status.is_authenticated === false);
  profiles.submit("test", "manual");
  await awaitingSecret.promise;
  profiles.submit("test", "fixture-code");
  const result = await login;
  assert.ok(isRecord(result) && result.is_authenticated === true);
  assert.ok(phases.some(phase => phase.phase === "polling"));
  assert.equal(phases.at(-1)?.phase, "completed");
  assert.equal(JSON.stringify(result).includes(credential.access), false);
  const directory = join(paths.root, "agent/auth");
  const files = (await readdir(directory)).filter(file => file.endsWith(".json"));
  assert.equal(files.length, 1);
  assert.match(files[0]!, /^[0-9a-f]{64}\.json$/);
  const storedPath = join(directory, files[0]!);
  assert.equal((await stat(directory)).mode & 0o777, 0o700);
  assert.equal((await stat(storedPath)).mode & 0o777, 0o600);
  const stored: unknown = JSON.parse(await readFile(storedPath, "utf8"));
  assert.ok(isRecord(stored) && stored.provider === "openai");
  const agentDir = join(paths.root, "agent");
  const deviceIdBefore = await readFile(join(directory, "device-id"), "utf8");
  const session = await openCanonicalSession(agentDir, paths.workspace);
  session.appendMessage({ role: "user", content: "Private transcript to purge", timestamp: Date.now() });
  for (const entry of await readdir(agentDir)) {
    if (entry !== "auth") await rm(join(agentDir, entry), { recursive: true });
  }
  assert.deepEqual(await readdir(agentDir), ["auth"]);
  const reopened = await ModelProfiles.create(config, join(paths.root, "agent"), () => {}, new Redactor());
  t.after(() => reopened.close());
  const restored = await reopened.status("test");
  assert.ok(isRecord(restored) && restored.is_authenticated === true);
  await writeFile(storedPath, "{invalid credential");
  await assert.rejects(reopened.status("test"), /Invalid authority-scoped OAuth file/);
  await reopened.logout("test");
  const signedOut = await reopened.status("test");
  assert.ok(isRecord(signedOut) && signedOut.is_authenticated === false);
  assert.equal((await readdir(directory)).filter(file => file.endsWith(".json")).length, 0);
  assert.equal(await readFile(join(directory, "device-id"), "utf8"), deviceIdBefore);
  const newSession = await openCanonicalSession(agentDir, paths.workspace);
  assert.notEqual(newSession.getSessionId(), session.getSessionId());
});

test("signing out makes the removal of the stored OAuth credential durable", async t => {
  const paths = await fixture(t);
  const credentials = new AuthorityCredentials(join(paths.root, "agent"), new Redactor());
  credentials.register("bobe-profile:test", "openai", true);
  await credentials.modify("bobe-profile:test", async () => ({ ...credential }));
  const directory = join(paths.root, "agent/auth");
  const [stored] = (await readdir(directory)).filter(file => file.endsWith(".json"));
  assert.ok(stored);
  const operations: string[] = [];
  const { open, unlink } = fsPromises;
  t.mock.method(fsPromises, "unlink", async (path: PathLike) => {
    operations.push(`unlink ${String(path)}`);
    await unlink(path);
  });
  t.mock.method(fsPromises, "open", async (path: PathLike, flags?: string | number, mode?: Mode) => {
    const handle = await open(path, flags, mode);
    const sync = handle.sync.bind(handle);
    handle.sync = async () => {
      operations.push(`sync ${String(path)}`);
      await sync();
    };
    return handle;
  });
  syncBuiltinESMExports();
  try {
    await credentials.delete("bobe-profile:test");
  } finally {
    t.mock.restoreAll();
    syncBuiltinESMExports();
  }
  assert.deepEqual(operations, [`unlink ${join(directory, stored)}`, `sync ${directory}`]);
  assert.equal(await credentials.read("bobe-profile:test"), undefined);
});

test("OAuth cancel rejects an outstanding callback and settles login without a credential", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  config.profiles.test = { kind: "open_ai", auth_method: "oauth" };
  const input = deferred<void>();
  const phases: AuthPhase[] = [];
  const profiles = await ModelProfiles.create(config, join(paths.root, "agent"), (_profile, phase) => {
    phases.push(phase);
    if (phase.phase === "awaiting_input") input.resolve();
  }, new Redactor());
  profiles.runtime.registerNativeProvider({
    ...controlledProvider(() => ({ text: "unused" })),
    auth: { oauth: {
      name: "Controlled cancel",
      login: async interaction => {
        await interaction.prompt({ type: "manual_code", message: "Code" });
        throw new Error("Canceled callback unexpectedly returned");
      },
      refresh: async () => ({ ...credential }),
      toAuth: async value => ({ apiKey: value.access }),
    } },
  });
  const login = profiles.login("test");
  const rejected = assert.rejects(login);
  await input.promise;
  await profiles.logout("test");
  await rejected;
  assert.equal(phases.at(-1)?.phase, "canceled");
  const status = await profiles.status("test");
  assert.ok(isRecord(status) && status.is_authenticated === false);
  await profiles.close();
});

test("ambient API keys do not authenticate unconfigured owner profiles", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  config.profiles.test!.api_key = null;
  const previous = process.env.OPENAI_API_KEY;
  process.env.OPENAI_API_KEY = "ambient-test-key-must-not-be-used";
  try {
    const profiles = await ModelProfiles.create(config, join(paths.root, "agent"), () => {}, new Redactor());
    const status = await profiles.status("test");
    assert.ok(isRecord(status) && status.is_authenticated === false);
    assert.equal(await profiles.runtime.checkAuth("openai"), undefined);
    await profiles.close();
  } finally {
    if (previous === undefined) delete process.env.OPENAI_API_KEY;
    else process.env.OPENAI_API_KEY = previous;
  }
});

test("Anthropic consumer OAuth is rejected before a third-party login can start", async t => {
  const paths = await fixture(t);
  const config = configuration(paths);
  for (const profile of [
    { kind: "anthropic" as const, auth_method: "oauth" as const },
    { kind: "anthropic" as const, auth_method: "api_key" as const, api_key: "sk-ant-oat01-fixture" },
    { kind: "anthropic" as const, auth_method: "api_key" as const, api_key: "sk-ant-ort01-fixture" },
  ]) {
    config.profiles.test = profile;
    await assert.rejects(
      ModelProfiles.create(config, join(paths.root, "agent"), () => {}, new Redactor()),
      /Anthropic API key/,
    );
  }
});
