import { ModelRuntime } from "@earendil-works/pi-coding-agent";
import {
  createAssistantMessageEventStream, createProvider, getSupportedThinkingLevels,
  type Api, type AssistantMessage, type AssistantMessageEventStream,
  type AuthEvent, type AuthPrompt, type Credential, type Model, type Provider,
  type ProviderAuth, type RefreshModelsContext, type ThinkingLevelMap,
} from "@earendil-works/pi-ai";
import { anthropicProvider } from "@earendil-works/pi-ai/providers/anthropic";
import { deepseekProvider } from "@earendil-works/pi-ai/providers/deepseek";
import { githubCopilotProvider } from "@earendil-works/pi-ai/providers/github-copilot";
import { openaiProvider } from "@earendil-works/pi-ai/providers/openai";
import { openrouterProvider } from "@earendil-works/pi-ai/providers/openrouter";
import { openAICompletionsApi } from "@earendil-works/pi-ai/api/openai-completions.lazy";
import { openAIResponsesApi } from "@earendil-works/pi-ai/api/openai-responses.lazy";
import { anthropicMessagesApi } from "@earendil-works/pi-ai/api/anthropic-messages.lazy";
import { AuthorityCredentials, deviceId } from "./credentials.js";
import { Cancelled, Redactor, Serial, assertActive } from "./errors.js";
import { isRecord, type AuthPhase, type Initialize, type Json, type Profile, type Route } from "./protocol.js";
import { ExecutionControl } from "./execution.js";

const anonymousLocalKey = "bobe-local-no-auth";
const noAmbientAuth = {
  env: async (): Promise<undefined> => undefined,
  fileExists: async (): Promise<boolean> => false,
};

interface ProfileRuntime {
  id: string;
  providerId: string;
  profile: Profile;
  provider: Provider;
  dynamic: boolean;
  metadata: Map<string, { contextWindow: number | null }>;
  catalogQueue: Serial;
}

interface WaitingInput {
  resolve(value: string): void;
  reject(error: Error): void;
  options: readonly string[] | undefined;
}

interface Login {
  controller: AbortController;
  completion: Promise<Json>;
  input: WaitingInput | undefined;
  url: string;
  code: string;
}

function failedMessage(model: Model<Api>, error: unknown): AssistantMessage {
  return {
    role: "assistant", content: [], api: model.api, provider: model.provider, model: model.id,
    stopReason: "error", errorMessage: error instanceof Error ? error.message : "Provider stream failed",
    timestamp: Date.now(),
    usage: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } },
  };
}

function scopedStream(source: AssistantMessageEventStream, model: Model<Api>): AssistantMessageEventStream {
  const output = createAssistantMessageEventStream();
  void (async () => {
    try {
      for await (const event of source) {
        if ("partial" in event) event.partial.provider = model.provider;
        if ("message" in event) event.message.provider = model.provider;
        if ("error" in event) event.error.provider = model.provider;
        output.push(event);
      }
      const result = await source.result();
      result.provider = model.provider;
      output.end(result);
    } catch (error) {
      const message = failedMessage(model, error);
      output.push({ type: "error", reason: "error", error: message });
      output.end(message);
    }
  })();
  return output;
}

function scopedProvider(base: Provider, id: string, profile: Profile): Provider {
  if (profile.kind === "anthropic" && (profile.auth_method === "oauth" ||
    profile.api_key?.includes("sk-ant-oat") || profile.api_key?.includes("sk-ant-ort"))) {
    throw new Error("Anthropic does not permit Claude subscription login in third-party applications; configure an Anthropic API key");
  }
  if (profile.auth_method === "oauth" && !base.auth.oauth) {
    throw new Error(`Provider ${profile.kind} does not support native pi OAuth`);
  }
  if (profile.auth_method === "oauth" && (profile.api_key || profile.base_url || profile.organization)) {
    throw new Error("Native OAuth profiles cannot override provider endpoints, keys, or organization");
  }
  const apiKey = base.auth.apiKey;
  const auth: ProviderAuth = profile.auth_method === "oauth"
    ? { oauth: base.auth.oauth! }
    : {
      apiKey: {
        name: `${profile.kind} API key`,
        resolve: async input => apiKey?.resolve({ ...input, ctx: noAmbientAuth }),
        check: async input => {
          if (apiKey?.check) return apiKey.check({ ...input, ctx: noAmbientAuth });
          const resolved = await apiKey?.resolve({ ...input, ctx: noAmbientAuth });
          return resolved ? { type: "api_key", source: "owner profile" } : undefined;
        },
      },
    };
  const project = (model: Model<Api>): Model<Api> => ({
    ...model,
    provider: id,
    ...(profile.base_url && profile.kind !== "rail_relay" && profile.kind !== "open_ai_compatible"
      ? { baseUrl: normalizeBaseUrl(profile.base_url) } : {}),
    ...(profile.organization ? { headers: { ...model.headers, "OpenAI-Organization": profile.organization } } : {}),
  });
  const native = <T extends Api>(model: Model<T>): Model<T> => ({ ...model, provider: base.id });
  return {
    id,
    name: base.name,
    ...(base.baseUrl ? { baseUrl: profile.base_url ? normalizeBaseUrl(profile.base_url) : base.baseUrl } : {}),
    auth,
    getModels: () => base.getModels().map(project),
    ...(base.filterModels ? {
      filterModels: (models: readonly Model<Api>[], credential: Credential | undefined) =>
        base.filterModels!(models, credential),
    } : {}),
    ...(base.refreshModels ? {
      refreshModels: (context: RefreshModelsContext) => base.refreshModels!(context),
    } : {}),
    stream: (model, context, options) => scopedStream(base.stream(native(model), context, options), model),
    streamSimple: (model, context, options) => scopedStream(base.streamSimple(native(model), context, options), model),
  };
}

function normalizeBaseUrl(input: string): string {
  const url = new URL(input);
  if (!["https:", "http:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) {
    throw new Error("Provider base URL must be an HTTP(S) URL without credentials, query, or fragment");
  }
  if (url.protocol === "http:" && !["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)) {
    throw new Error("Non-loopback inference providers require HTTPS");
  }
  return url.toString().replace(/\/$/, "");
}

function positiveInteger(value: unknown): number | undefined {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0 ? value : undefined;
}

type CompatibleApi = "anthropic-messages" | "openai-responses" | "openai-completions";

interface DiscoveredModel {
  model: Model<CompatibleApi>;
  contextWindow: number | null;
}

function minimumPositive(...values: unknown[]): number | undefined {
  const limits = values.flatMap(value => {
    const positive = positiveInteger(value);
    return positive === undefined ? [] : [positive];
  });
  return limits.length ? Math.min(...limits) : undefined;
}

function discoveredModel(raw: unknown, provider: string, baseUrl: string, relay: boolean): DiscoveredModel {
  if (!isRecord(raw) || typeof raw.id !== "string" || !raw.id) throw new Error("Invalid provider model catalog entry");
  const api = raw.api ?? (relay ? undefined : "chat");
  if (api !== "messages" && api !== "responses" && api !== "chat") {
    throw new Error("Provider catalog api must be messages, responses, or chat");
  }
  const limits = isRecord(raw.top_provider) ? raw.top_provider : {};
  const architecture = isRecord(raw.architecture) ? raw.architecture : {};
  const capabilities = isRecord(raw.capabilities) ? raw.capabilities : {};
  const vision = raw.vision === true ||
    capabilities.image_input === true ||
    (Array.isArray(capabilities.input_modalities) && capabilities.input_modalities.includes("image")) ||
    (Array.isArray(architecture.input_modalities) && architecture.input_modalities.includes("image"));
  const efforts = Array.isArray(capabilities.effort)
    ? capabilities.effort.filter((value): value is string => typeof value === "string") : undefined;
  let thinkingLevelMap: ThinkingLevelMap | undefined;
  if (efforts?.length) {
    thinkingLevelMap = {};
    for (const level of ["off", "minimal", "low", "medium", "high", "xhigh", "max"] as const) {
      const wire = level === "off" && efforts.includes("none") ? "none" : level;
      thinkingLevelMap[level] = efforts.includes(wire) ? wire : null;
    }
  }
  const knownWindow = minimumPositive(
    raw.context_window, raw.context_length, raw.max_input_tokens, raw.max_prompt_tokens,
  );
  const contextWindow = knownWindow ?? 8_192;
  const maxTokens = Math.min(
    positiveInteger(raw.max_output_tokens) ?? positiveInteger(limits.max_completion_tokens) ?? 2_048,
    Math.max(1, Math.floor(contextWindow / 2)),
  );
  const common: Omit<Model<CompatibleApi>, "api" | "baseUrl" | "compat"> = {
    id: raw.id,
    name: typeof raw.name === "string" ? raw.name : raw.id,
    provider,
    input: vision ? ["text", "image"] : ["text"],
    reasoning: raw.reasoning === true || capabilities.reasoning === true ||
      Boolean(efforts?.some(value => !["none", "off"].includes(value))),
    ...(thinkingLevelMap ? { thinkingLevelMap } : {}),
    contextWindow, maxTokens,
    cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 },
  };
  const model: Model<CompatibleApi> = api === "messages"
    ? { ...common, api: "anthropic-messages", baseUrl: baseUrl.replace(/\/v1$/, "") }
    : api === "responses"
      ? { ...common, api: "openai-responses", baseUrl }
      : { ...common, api: "openai-completions", baseUrl, compat: { supportsStore: false } };
  return { model, contextWindow: knownWindow ?? null };
}

async function catalogView(baseUrl: string, headers: Headers, signal: AbortSignal): Promise<unknown[]> {
  const response = await fetch(`${baseUrl}/models`, {
    headers, signal, redirect: "error",
  });
  if (!response.ok) throw new Error(`Provider model catalog returned HTTP ${response.status}`);
  const body: unknown = await response.json();
  if (!isRecord(body) || !Array.isArray(body.data)) throw new Error("Provider returned an invalid model catalog");
  return body.data;
}

async function ollamaCatalog(baseUrl: string, headers: Headers, signal: AbortSignal): Promise<unknown[]> {
  const root = baseUrl.replace(/\/v1$/, "");
  const tags = await fetch(`${root}/api/tags`, { headers, signal, redirect: "error" });
  if (!tags.ok) throw new Error(`Ollama model catalog returned HTTP ${tags.status}`);
  const body: unknown = await tags.json();
  if (!isRecord(body) || !Array.isArray(body.models)) throw new Error("Ollama returned an invalid model catalog");
  const models: unknown[] = [];
  for (const entry of body.models) {
    assertActive(signal);
    if (!isRecord(entry) || typeof entry.name !== "string" || !entry.name) {
      throw new Error("Ollama returned a model without an identifier");
    }
    const detailHeaders = new Headers(headers);
    detailHeaders.set("Content-Type", "application/json");
    const response = await fetch(`${root}/api/show`, {
      method: "POST", headers: detailHeaders, signal, redirect: "error",
      body: JSON.stringify({ model: entry.name }),
    });
    if (!response.ok) throw new Error(`Ollama model details returned HTTP ${response.status}`);
    const details: unknown = await response.json();
    if (!isRecord(details) || !Array.isArray(details.capabilities)) {
      throw new Error("Ollama returned invalid model capabilities");
    }
    const info = isRecord(details.model_info) ? details.model_info : {};
    const context = minimumPositive(...Object.entries(info)
      .filter(([key]) => key.endsWith(".context_length")).map(([, value]) => value));
    models.push({
      id: entry.name, name: entry.name, api: "chat",
      vision: details.capabilities.includes("vision"),
      ...(context ? { context_window: context } : {}),
    });
  }
  return models;
}

function compatibleProvider(
  id: string, profile: Profile, metadata: ProfileRuntime["metadata"],
): Provider {
  if (profile.auth_method !== "api_key") throw new Error("Compatible providers require an owner-supplied API token");
  if (!profile.base_url) throw new Error("Compatible provider requires base_url");
  const relay = profile.kind === "rail_relay";
  const configuredBaseUrl = normalizeBaseUrl(profile.base_url);
  const baseUrl = relay && !configuredBaseUrl.endsWith("/v1") ? `${configuredBaseUrl}/v1` : configuredBaseUrl;
  if (relay && profile.offline) throw new Error("Rail Relay is an external inference provider");
  return createProvider({
    id, name: profile.kind === "rail_relay" ? "Rail Relay" : "OpenAI-compatible",
    baseUrl,
    auth: {
      apiKey: {
        name: "Owner provider token",
        resolve: async ({ credential }) => credential?.key
          ? { auth: { apiKey: credential.key }, source: "owner profile" }
          : undefined,
      },
    },
    models: [],
    api: {
      "openai-completions": openAICompletionsApi(),
      "openai-responses": openAIResponsesApi(),
      "anthropic-messages": anthropicMessagesApi(),
    },
    fetchModels: async context => {
      assertActive(context.signal);
      if (!context.credential || context.credential.type !== "api_key" || !context.credential.key) {
        throw new Error("Provider token is not configured");
      }
      const headers = new Headers({ Accept: "application/json" });
      if (context.credential.key !== anonymousLocalKey) headers.set("Authorization", `Bearer ${context.credential.key}`);
      if (profile.organization) headers.set("OpenAI-Organization", profile.organization);
      const controller = new AbortController();
      const signal = AbortSignal.any([context.signal, controller.signal, AbortSignal.timeout(20_000)]);
      const views = [profile.local_runtime === "ollama"
        ? ollamaCatalog(baseUrl, headers, signal)
        : catalogView(baseUrl, headers, signal)];
      if (relay) {
        views.push(catalogView(baseUrl, new Headers({
          Accept: "application/json",
          "x-api-key": context.credential.key,
          "anthropic-version": "2023-06-01",
        }), signal));
      }
      let catalogs: unknown[][];
      try {
        catalogs = await Promise.all(views);
      } finally {
        controller.abort();
        await Promise.allSettled(views);
      }
      const discovered = new Map<string, DiscoveredModel>();
      for (const raw of catalogs.flat()) {
        const entry = discoveredModel(raw, id, baseUrl, relay);
        if (!discovered.has(entry.model.id)) discovered.set(entry.model.id, entry);
      }
      if (discovered.size === 0) throw new Error("Provider returned an empty model catalog");
      metadata.clear();
      for (const { model, contextWindow } of discovered.values()) {
        metadata.set(model.id, { contextWindow });
      }
      return [...discovered.values()].map(entry => entry.model);
    },
  });
}

function nativeProvider(profile: Profile, metadata: ProfileRuntime["metadata"]): Provider {
  switch (profile.kind) {
    case "copilot":
      if (profile.auth_method !== "oauth") throw new Error("GitHub Copilot requires native OAuth");
      return githubCopilotProvider();
    case "open_ai": return openaiProvider();
    case "anthropic": return anthropicProvider();
    case "deepseek": return deepseekProvider();
    case "open_router": return openrouterProvider();
    case "rail_relay":
    case "open_ai_compatible": return compatibleProvider(profile.kind, profile, metadata);
  }
}

export class ModelProfiles {
  private readonly profiles = new Map<string, ProfileRuntime>();
  private readonly logins = new Map<string, Login>();
  private readonly logouts = new Set<string>();

  private constructor(
    readonly runtime: ModelRuntime,
    private readonly credentials: AuthorityCredentials,
    private readonly installationId: string,
    private readonly notify: (profile: string, phase: AuthPhase) => void,
    private readonly redactor: Redactor,
    readonly execution: ExecutionControl,
  ) {}

  static async create(
    config: Initialize,
    agentDir: string,
    notify: (profile: string, phase: AuthPhase) => void,
    redactor: Redactor,
    signal?: AbortSignal,
  ): Promise<ModelProfiles> {
    const credentials = new AuthorityCredentials(agentDir, redactor);
    const runtime = await ModelRuntime.create({
      credentials, modelsPath: null, allowModelNetwork: false, refreshOnCreate: false,
      ...(signal ? { signal } : {}),
    });
    for (const provider of runtime.getProviders()) {
      runtime.registerNativeProvider({
        ...provider,
        auth: { apiKey: { name: "Unconfigured provider", resolve: async () => undefined } },
      });
    }
    const result = new ModelProfiles(runtime, credentials, await deviceId(agentDir), notify, redactor, new ExecutionControl(config));
    for (const [id, profile] of Object.entries(config.profiles)) {
      const providerId = `bobe-profile:${id}`;
      const metadata: ProfileRuntime["metadata"] = new Map();
      const base = nativeProvider(profile, metadata);
      const dynamic = profile.kind === "rail_relay" || profile.kind === "open_ai_compatible";
      const key = profile.api_key ||
        (profile.kind === "open_ai_compatible" && profile.offline ? anonymousLocalKey : undefined);
      credentials.register(providerId, base.id, profile.auth_method === "oauth", key);
      const provider = scopedProvider(base, providerId, profile);
      result.registerNativeProvider(provider);
      result.profiles.set(id, { id, providerId, profile, provider, dynamic, metadata, catalogQueue: new Serial() });
    }
    for (const route of Object.values(config.routes)) result.get(route.profile);
    return result;
  }

  registerNativeProvider(provider: Provider): void {
    this.runtime.registerNativeProvider(this.execution.provider(provider));
  }

  private get(id: string): ProfileRuntime {
    const profile = this.profiles.get(id);
    if (!profile) throw new Error(`Unknown provider profile: ${id}`);
    return profile;
  }

  async status(id: string, signal?: AbortSignal): Promise<Json> {
    const profile = this.get(id);
    const options = signal ? { signal } : undefined;
    const check = await this.runtime.checkAuth(profile.providerId, options);
    const credential = await this.credentials.read(profile.providerId, options);
    const login = credential?.type === "oauth" && typeof credential.login === "string" ? credential.login : null;
    return {
      is_authenticated: check !== undefined,
      auth_type: check?.type ?? profile.profile.auth_method,
      host: profile.provider.baseUrl ? new URL(profile.provider.baseUrl).host : null,
      login,
      status_message: check
        ? check.type === "oauth" ? "Native pi OAuth configured; access is checked when used" : "Owner API profile configured"
        : "Authentication is not configured",
    };
  }

  private async refresh(profile: ProfileRuntime, signal?: AbortSignal): Promise<void> {
    if (!profile.dynamic) return;
    const refreshed = await this.runtime.refresh({
      providers: [profile.providerId], allowNetwork: true, force: true,
      ...(signal ? { signal } : {}),
    });
    assertActive(signal);
    const error = refreshed.errors.get(profile.providerId);
    if (error) throw error;
    if (refreshed.aborted) throw new Cancelled();
    if (this.runtime.getModels(profile.providerId).length === 0) {
      throw new Error("Provider has no discovered models; configure authentication and check its catalog endpoint");
    }
  }

  async models(id: string, signal?: AbortSignal): Promise<Json> {
    const profile = this.get(id);
    return profile.catalogQueue.run(() => this.listModels(profile, signal));
  }

  private async listModels(profile: ProfileRuntime, signal?: AbortSignal): Promise<Json> {
    assertActive(signal);
    await this.refresh(profile, signal);
    const configured = await this.runtime.checkAuth(profile.providerId, signal ? { signal } : undefined);
    const models = configured
      ? await this.runtime.getAvailable(profile.providerId, signal ? { signal } : undefined)
      : this.runtime.getModels(profile.providerId);
    return models.map(model => {
      const efforts = getSupportedThinkingLevels(model);
      return {
        id: model.id, name: model.name, vision: model.input.includes("image"),
        context_window: profile.dynamic ? profile.metadata.get(model.id)?.contextWindow ?? null : model.contextWindow,
        default_reasoning_effort: model.reasoning ? (efforts.includes("medium") ? "medium" : efforts[0] ?? "off") : "off",
        supported_reasoning_efforts: efforts,
      };
    });
  }

  async select(route: Route, signal?: AbortSignal): Promise<Model<Api>> {
    const profile = this.get(route.profile);
    return profile.catalogQueue.run(async () => this.execution.limitModel(await this.selectModel(profile, route, signal)));
  }

  private async selectModel(profile: ProfileRuntime, route: Route, signal?: AbortSignal): Promise<Model<Api>> {
    if (!route.model) throw new Error(`Select a model for provider profile ${route.profile}`);
    assertActive(signal);
    if (!await this.runtime.checkAuth(profile.providerId, signal ? { signal } : undefined)) {
      throw new Error(`Authenticate provider profile ${route.profile}`);
    }
    let model = this.runtime.getModel(profile.providerId, route.model);
    if (!model && profile.dynamic) {
      await this.refresh(profile, signal);
      model = this.runtime.getModel(profile.providerId, route.model);
    }
    if (!model) throw new Error(`Model ${route.model} is not in the selected provider catalog`);
    const available = await this.runtime.getAvailable(profile.providerId, signal ? { signal } : undefined);
    if (!available.some(candidate => candidate.id === model.id)) throw new Error("Selected model is not available to this account");
    return model;
  }

  login(id: string, signal?: AbortSignal): Promise<Json> {
    const profile = this.get(id);
    if (profile.profile.auth_method !== "oauth") throw new Error("Set API keys through the daemon profile configuration");
    if (this.logins.has(id)) throw new Error("Provider login is already active");
    if (this.logouts.has(id)) throw new Error("Provider sign-out is in progress");
    const controller = new AbortController();
    const combined = signal ? AbortSignal.any([signal, controller.signal]) : controller.signal;
    const login: Login = { controller, completion: Promise.resolve(null), input: undefined, url: "", code: "" };
    this.logins.set(id, login);
    this.notify(id, { phase: "preparing" });
    login.completion = this.performLogin(profile, login, combined);
    return login.completion;
  }

  private async performLogin(profile: ProfileRuntime, login: Login, signal: AbortSignal): Promise<Json> {
    try {
      await this.runtime.login(profile.providerId, "oauth", {
        signal,
        prompt: prompt => this.prompt(profile.id, login, prompt, signal),
        notify: event => this.authEvent(profile.id, login, event),
      }, { getDeviceId: () => this.installationId });
      assertActive(signal);
      const status = await this.status(profile.id, signal);
      this.notify(profile.id, { phase: "completed" });
      return status;
    } catch (error) {
      this.notify(profile.id, signal.aborted
        ? { phase: "canceled" }
        : { phase: "failed", message: this.redactor.message(error) });
      throw error;
    } finally {
      login.input?.reject(new Cancelled("Authentication ended"));
      this.logins.delete(profile.id);
    }
  }

  private prompt(id: string, login: Login, prompt: AuthPrompt, signal: AbortSignal): Promise<string> {
    const combined = prompt.signal ? AbortSignal.any([signal, prompt.signal]) : signal;
    assertActive(combined);
    if (login.input) throw new Error("Provider requested overlapping authentication input");
    const options = prompt.type === "select" ? prompt.options.map(option => option.id) : undefined;
    const message = prompt.type === "select"
      ? `${prompt.message}\n${prompt.options.map(option => `${option.id}: ${option.label}`).join("\n")}`
      : prompt.message;
    return new Promise<string>((resolve, reject) => {
      const cleanup = (): void => {
        combined.removeEventListener("abort", onAbort);
        login.input = undefined;
      };
      const onAbort = (): void => { cleanup(); reject(new Cancelled()); };
      login.input = {
        options,
        resolve: value => { cleanup(); resolve(value); },
        reject: error => { cleanup(); reject(error); },
      };
      combined.addEventListener("abort", onAbort, { once: true });
      this.notify(id, { phase: "awaiting_input", message, password: prompt.type === "secret" });
    });
  }

  private authEvent(id: string, login: Login, event: AuthEvent): void {
    if (event.type === "auth_url") {
      login.url = event.url;
      this.notify(id, { phase: "awaiting_user", url: login.url, code: login.code });
    } else if (event.type === "device_code") {
      login.url = event.verificationUri;
      login.code = event.userCode;
      this.notify(id, { phase: "awaiting_user", url: login.url, code: login.code });
    } else if (event.type === "progress") {
      if (login.url) this.notify(id, { phase: "polling", url: login.url, code: login.code });
      else if (!login.input) this.notify(id, { phase: "preparing" });
    } else if (event.links?.[0]) {
      login.url = event.links[0].url;
      this.notify(id, { phase: "awaiting_user", url: login.url, code: login.code });
    }
  }

  submit(id: string, value: string): Json {
    this.get(id);
    const input = this.logins.get(id)?.input;
    if (!input) throw new Error("Provider is not waiting for authentication input");
    if (input.options && !input.options.includes(value)) throw new Error("Select one of the offered authentication option IDs");
    input.resolve(value);
    return {};
  }

  async cancel(id: string): Promise<Json> {
    this.get(id);
    const login = this.logins.get(id);
    if (!login) return {};
    login.controller.abort();
    await Promise.allSettled([login.completion]);
    return {};
  }

  async logout(id: string, signal?: AbortSignal): Promise<Json> {
    const profile = this.get(id);
    if (profile.profile.auth_method !== "oauth") throw new Error("API keys are owned by the daemon");
    if (this.logouts.has(id)) throw new Error("Provider sign-out is already in progress");
    this.logouts.add(id);
    try {
      await this.cancel(id);
      await this.runtime.logout(profile.providerId, signal ? { signal } : undefined);
      return {};
    } finally {
      this.logouts.delete(id);
    }
  }

  async close(): Promise<void> {
    const active = [...this.logins.values()];
    for (const login of active) login.controller.abort();
    await Promise.allSettled(active.map(login => login.completion));
  }
}
