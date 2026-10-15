# BoBe pi execution worker

This private Node process embeds `@earendil-works/pi-coding-agent` 1.2.0. Rust
owns admission, identity, memory, goals, skills, secrets, voice and body state;
the worker owns pi execution and its scoped resources.

## Build and checks

Node 22.19.0 or later:

```sh
npm ci --prefix BoBeAgent --ignore-scripts --no-audit --no-fund
npm run build --prefix BoBeAgent
npm run check --prefix BoBeAgent
npm test --prefix BoBeAgent
```

The entry point is `dist/main.js`. Ship `dist/`, `assets/`, `package.json`, the
lockfile, production dependencies and the pinned Node together.
`assets/chat.md` is the single behavior source, also embedded by Rust. Rust
writes the authority's `skills/chat/SKILL.md`, and pi reads that skill, with owner
edits, for each companion turn. Never replace it with pi's coding-assistant
prompt or a separate persona.

Search uses the pinned `rg` and `fd` packaged in `bin/` beside `dist/`; the entry
point prepends that path. Developer runs may use installed `rg` and `fd`
(`fdfind` on Linux). First-use SDK downloads are disabled, so a missing binary is
a tool error. SDK imports happen only after initialization selects the owner
agent directory, including pi's import-time tool cache.

`src/search.ts` adapts pi 1.2.0 search so `rg` and `fd`, like shell and MCP,
start under Rust's process owner and capacity. Tool schemas and truncation
helpers stay upstream and find uses its operations hook; grep has no process
hook, so its bounded formatting and awaited cancellation are maintained here.
The adapted code ([pi v1.2.0 tools](https://github.com/earendil-works/pi/tree/v1.2.0/packages/coding-agent/src/core/tools))
is MIT, copyright 2025 Mario Zechner; `assets/pi-search.LICENSE` ships with it.

Tests use real pi `AgentSession` objects, controlled provider adapters, a local
HTTP fixture and real shell and MCP subprocesses. They are regression tests, not
product qualification: operate the daemon and `bobe` CLI with a real provider for
authentication, execution, persistence and restart evidence.

## Protocol

`src/protocol.ts` validates it. Each stdin or stdout line is one UTF-8 JSON
object of at most 16 MiB, and stdout carries only protocol frames. Rust request
IDs are positive safe integers and worker callback IDs negative. Requests run
concurrently and replies may arrive out of order. Malformed input, duplicate
request IDs and unknown callback IDs fail. `fixtures/host-contract-v1.json` is
the shared fixture for Rust contract tests; `src/layout.ts` defines runtime
paths.

Requests are `{id, method, params}`, replies `{id, result}` or
`{id, error: {message}}`, notifications `{method, params}`.

| Method | Parameters | Result |
| --- | --- | --- |
| `initialize` | Protocol v1 owner paths, profiles, routes, exported tool schemas, capability instructions, MCP servers, enabled tools, execution limits and all workload budgets | `{protocol_version:1,harness:"pi",version:"1.2.0"}` |
| `chat` | `{session_key?,text,channel:"text"\|"voice",context,memory_context}` | `{}` after settled success |
| `background` | `{job_id,title,task,context,memory_context,attempt,route}` | `{session_id,output}` after the task and its resources settle |
| `auxiliary` | `{class:"goals"\|"small"\|"consolidate",job:{job_id,kind,instructions,input},context}` | `{job_id,output,error}`; failed output is `null`, never a fabricated object |
| `probe` | Optional `{route}`; defaults to active chat | `{model_execution,streaming,tool_execution,error}` from a finite echo-tool check |
| `context_status` | Optional `{session_key}` | Channel pi session ID, context estimate, token and cache counters, tool totals and compaction count |
| `compact` | Optional `{session_key,keep_recent_tokens}` | Native pi summary, retained-entry ID and before and after estimates |
| `abort` | `{request_id}` | `{}` only after the operation settles |
| `steer` | `{request_id,text,follow_up}` | `{queued:true}` for an active run; pi applies guidance at a tool boundary or after the current task |
| `reload` | Optional `{mcp,skill_directories}` | `{}` |
| `mcp_status` | `{}` | `[{name,connected,status,error}]` |
| `mcp_tools` | `{server_name}` | `[{name,description}]` without excluded tools |
| `auth_status` | `{profile}` | `{is_authenticated,auth_type,host,login,status_message}` |
| `models` | `{profile}` | `[{id,name,vision,context_window,default_reasoning_effort,supported_reasoning_efforts}]` |
| `auth_login` | `{profile}` | Authentication status after the native OAuth flow |
| `auth_submit` | `{profile,value}` | `{}` |
| `auth_cancel` | `{profile}` | `{}` after login settles |
| `auth_logout` | `{profile}` | `{}` after login settles and pi removes the authority-scoped OAuth credential |
| `shutdown` | `{}` | `{}`, then exit |

Route reasoning values are pi's `off`, `minimal`, `low`, `medium`, `high`,
`xhigh` and `max`; `null` selects the worker default.

Execution notifications are `{method:"event",params:{request_id,delta}}` with
deltas `message_delta {message_id,text}`, `message_complete
{message_id,content,output_tokens}`, `tool_start {id,name}`, `tool_complete
{id,name,success}`, `error {message}` and `done`. Parent assistant text streams
live with stable per-message IDs. Thinking, child text and tool arguments are
never sent as reply text, and failed or truncated text stays partial. `done`
follows only after pi and persistence settle. Provider retries are disabled, and
overflow replay is blocked after visible output.

`bobe agent status --json` shows these counters and `bobe agent compact` requests
native compaction, which holds the shared admission permit and so never
interrupts another owner turn; `--keep-recent-tokens` changes only that
compaction's retained tail.

Prompt caching uses pi's provider adapters; cache warming is off to avoid
unsolicited inference. Behavior, tool definitions and memory form the stable
prefix; time and channel hints are appended with the new user turn. A changed
memory, skill or tool definition invalidates its part of the prefix. Cache reads
and writes are reported by the provider, never inferred. Provider tool-call IDs
stay in the private transcript; owner events carry opaque correlated IDs without
reasoning signatures.

`tool_activity {request_id,name}` precedes parent tool execution. Voice requests
get short speech guidance and pi's low reasoning, clamped to the model. Generic
interactive extension dialogs fail rather than wait unattended; important
questions use `bobe_owner_input` and the real surfaces.

Authentication notifications are `{method:"auth_event",params:{profile,phase}}`
with phases `preparing`, `awaiting_user {url,code}`, `polling {url,code}`,
`awaiting_input {message,password}`, `completed`, `failed {message}` and
`canceled`. Select prompts list option IDs in `message`, and `auth_submit.value`
must be one of them.

## Owner callbacks and cancellation

The worker calls `domain_tool` with `{request_id,name,arguments}` and the original
Rust JSON schema. Rust returns `{success:true,value}` or a failure; failed
`value.error`, `error` and `message` strings become tool errors, never successful
output.

Rust binds each domain callback to its admitted request, cancellation token and
authenticated channel. Channel chats, background jobs and their descendants get
domain tools; auxiliary and probe sessions do not. The worker keeps callback
waiters until Rust replies and does not acknowledge cancellation while owner
effects are uncertain; the daemon's bounded kill is the last resort. No tool call
is replayed automatically.

Each channel's companion may call `background_job` with
`{request_id,name,arguments,source_key?}` and actions `start`, `list`, `get` and
`cancel`. `start` needs a SHA-256 source identity derived from the pi session, the
assistant-message entry and the native tool-call ID, because provider IDs can
repeat across turns. Rust journals the task and returns at once without running
another model loop. Background sessions may inspect and cancel tasks but not
start immediate jobs. Owners can resume ordinary jobs; hourly goal review can
reopen an interrupted goal task to inspect effects and continue safely, never by
replaying a stored tool call.

`bobe_skill_create` returns after Rust publishes and validates the skill and the
worker reloads its resources; the nested reload is handled while the tool is
pending. If activation fails, Rust restores and reloads the previous file.
Success confirms publication and activation, with one skill owner.

MCP uses pi's native extension over a Rust-backed stdio transport. Rust supplies
`{command,args,env,timeout_ms,excluded_tools}` per server with environment values
already resolved; the worker neither evaluates shell expressions nor loads `.env`
files. Aborting an active MCP call closes its process, and the next turn
reconnects through pi without replacing the channel session. Reload reaches
every open channel; shutdown closes all session and MCP resources.

Shell, MCP, grep and find run as daemon children in private process groups.
Worker death cancels and reaps them before the run settles, and cold-start
initialization and shutdown keep ownership if their caller is cancelled. This is
not an OS sandbox: a daemon `SIGKILL` outside a service manager, intentional
daemonization and hostile grandchildren are not contained.

`agent.execution` bounds inference, tools, processes and live channel objects;
[Admission, runs and resources](../docs/architecture.md#admission-runs-and-resources)
lists the defaults. A parent waiting for children holds no model permit.
Workload input limits use pi token estimates and output limits reach provider
`maxTokens`. Task budgets select timeouts and turn limits below owner ceilings.
Idle channel and MCP objects retire after the idle timeout or for cache capacity;
the channel pointer and transcript stay durable. Rust supplies every policy value
at initialization and missing limits fail validation; the worker has no fallback
defaults. The shared host fixture checks Rust serialization against the worker
schema.

`bobe_schedule` and `bobe_owner_input` keep their state in Rust; see
[Durable jobs, schedules and important questions](../docs/architecture.md#durable-jobs-schedules-and-important-questions)
and, for commitment and presentation receipts,
[Initiative and delivery](../docs/architecture.md#initiative-and-delivery).

## Persistence and configuration

Owner state lives only under `data_root/agent/`:

- `session.json`: `{version:1,session_id,file}` for the owner channel.
- `sessions/`: the owner transcript and channel and task directories.
- `sessions/channels/<channel-uuid>/session.json`: each enrolled channel's
  transcript.
- `sessions/children/`: scoped child transcripts linked by pi's `parentSession`.
- `sessions/jobs/<job-uuid>/session.json`: a durable job's transcript. Resume
  opens it and never replaces a channel conversation.
- `auth/<sha256(profile-provider-id)>.json`: native OAuth, with provider ID
  `bobe-profile:<profile-id>`.
- `auth/device-id`: the SDK installation identifier.
- `runtime-files/` and `mcp.log`: authority-local runtime artifacts.

All worker credentials and the installation identifier are in `auth/`, so a
privacy purge can keep exactly that directory and transcript backups can exclude
it.

The pointer is opened with `SessionManager.open`. First use creates a pi session
and publishes its header and pointer before accepting turns. Invalid entries, a
missing transcript or an orphan transcript without a pointer require owner
recovery; the worker never picks the most recent session or replaces damaged
history.

The nil UUID selects the owner pointer. Rust derives other channel UUIDs from
authenticated endpoint IDs, so reconnects and credential rotation keep their
channel. Every session gets the same workspace, memory, skills and credentials;
`bobe_history_recall` fetches bounded excerpts from another channel when needed.

Changing a provider or route restarts the worker with new initialization data
and the same channel pointers. Tool, skill and MCP refreshes never fork history.
Skill-only reload is safe inside a turn; MCP configuration reload needs idle
execution. `enabled_tools` is a restart-bound selection shared by every full
session, and disabled native tools stay excluded even if resumed history once
declared them.

API keys arrive only in the initialization request and stay in memory; OAuth
files are owner-scoped and private. No global pi auth, settings, prompts,
extensions or project `.pi` resources are loaded. The entry point removes
unrelated environment variables, and the Rust launcher removes `NODE_OPTIONS`
because Node reads it before the entry point runs.

## Models and bounded work

Rust supplies chat, goal-work, memory-consolidation and optional `small` routes.
Small decisions use `small` when configured and otherwise the goals route. With
`small`, manual and automatic compaction use pi's compactor on that model and keep
the channel pointer; without it, compaction uses the session model. Never choose
a model because its name sounds small: access, vision and context limits come
from the provider catalog and owner configuration.

OpenAI uses pi's API and native subscription OAuth, which stay separate and
account-specific. Anthropic uses its API-key implementation only: BoBe rejects
Claude subscription OAuth because
[Anthropic's policy](https://code.claude.com/docs/en/legal-and-compliance#authentication-and-credential-use)
does not permit third-party apps to offer Claude.ai login or relay consumer
subscription credentials, whatever upstream implements. GitHub Copilot,
DeepSeek and OpenRouter use pi's native providers. Rail Relay is an external
token-authenticated endpoint.

Profiles with `local_runtime: "ollama"` read `/api/tags` and `/api/show` for model
IDs, vision and context metadata, then execute through pi's OpenAI-compatible
transport; discovery is not a separate loop.

Generic providers discover IDs from `GET <base_url>/models` and execute through
pi's native transports. Relay accepts an origin or `/v1` base URL and reads both
`/v1/models` views, the Bearer view without Anthropic headers and the Messages
view with `x-api-key` and `anthropic-version`, using their union. A failed view
fails discovery rather than returning a partial catalog, and duplicate IDs keep
the Bearer entry so route IDs stay stable.

Relay entries must set `api: "messages" | "responses" | "chat"`, selecting pi's
Anthropic Messages, OpenAI Responses or OpenAI Chat Completions transport.
Messages uses the root base URL so the SDK adds `/v1/messages` once; the others
use the `/v1` base.

Catalog entries may supply `name`, `vision`, `context_window`, `context_length`,
`max_input_tokens`, `max_prompt_tokens` and `max_output_tokens`. The context bound
is the lowest positive input or context limit; zero, negative or missing values
never override a positive one, and unknown context is reported as `null`. Relay
`capabilities.image_input`, `reasoning` and `effort` carry the provider's vision
and thinking support. Catalog refresh and selection are serialized per profile,
so concurrent inspection cannot clear an active turn's model. An ID-only generic
catalog executes with 8,192 context tokens, 2,048 output tokens, and no vision or
reasoning. An unauthenticated local profile must be marked `offline`;
non-loopback HTTP endpoints are rejected.

Auxiliary jobs return their JSON value through `bobe_submit_result` or a final
`{"output": <value>}` envelope. Summaries may be strings; activation and
consolidation return objects. Goals and consolidation use the daemon's 3- and
15-minute deadlines, and probes are bounded to 90 seconds and must stream a real
echo-tool round.

The `subagent` tool follows pi's MIT-licensed
[`examples/extensions/subagent`](https://github.com/earendil-works/pi/tree/main/packages/coding-agent/examples/extensions/subagent)
semantics with in-process isolated SDK sessions instead of another CLI. Its input
is `{mode:"parallel"|"chain",tasks:[...]}`, and a one-item list runs one child,
which avoids conflicting optional shapes under strict JSON schemas. Children
inherit file, shell and search tools, domain tools, MCP, skills and the model,
with explicitly scoped context. Parallel lists take at most eight tasks and run
four at a time; a turn creates at most sixteen children across two delegation
levels. Each child is limited to ten minutes and 32 turns and settles with its
owner. Child transcripts are durable audit records, not resumable jobs, and never
replace a channel's conversation.

`background_task` queues work that outlives the reply. Rust admits and budgets
it ([limits](../docs/architecture.md#durable-jobs-schedules-and-important-questions));
nested children keep the budgets above. Background work uses the same `Sessions`
implementation, not pi-durable or a second loop, and returns results through
Rust admission to a channel. Goal work runs whether or not an app is open; the
daemon separately uses the small route to offer, defer or suppress an update.
Interruption records uncertainty, so resumption inspects effects instead of
repeating them.
