# BoBe pi execution worker

This private Node process embeds `@earendil-works/pi-coding-agent` 1.0.3.
Rust owns admission, identity, memory, goals, skills, secrets, voice, and body
state. The worker owns pi execution and its scoped resources.

## Build and checks

Use Node 22.19.0 or later:

```sh
npm ci --prefix BoBeAgent --ignore-scripts --no-audit --no-fund
npm run build --prefix BoBeAgent
npm run check --prefix BoBeAgent
npm test --prefix BoBeAgent
```

The private entry point is `BoBeAgent/dist/main.js`. Package `dist/`,
`assets/`, `package.json`, the lockfile, production dependencies, and the
pinned Node executable together. `assets/chat.md` is the single shipped
behavior source, also embedded by Rust. Rust prepares the
authority's `skills/chat/SKILL.md`; pi reads that effective owner skill for
each companion turn, including owner edits. Do not replace it with pi's
coding-assistant prompt or a separate worker persona.

Native pi search uses the pinned `rg` and `fd` executables packaged in `bin/`
beside `dist/`. The entry point prepends that private `bin/` path.
Developer runs can use installed `rg` and `fd` (`fdfind` on Linux).
Unpinned first-use SDK downloads are disabled; missing binaries produce
tool errors. SDK imports occur only after the private initialization has
selected the owner agent directory, including pi's import-time tool cache.

`src/search.ts` adapts pi 1.0.3 search execution and formatting so `rg` and
`fd`, like shell and MCP processes, start under Rust's process owner and
aggregate capacity. The native tool schemas and truncation helpers remain
upstream; find uses its operations hook. Pi's grep does not expose a process
hook, so its formatting is maintained here with explicit bounded output and
awaited cancellation. Source: [pi v1.0.3 tools](https://github.com/earendil-works/pi/tree/v1.0.3/packages/coding-agent/src/core/tools).
The adapted code is MIT-licensed, copyright 2025 Mario Zechner;
`assets/pi-search.LICENSE` travels with the worker.

Tests use real pi `AgentSession` objects, controlled provider adapters, a
bounded local HTTP fixture, and real native shell/MCP subprocesses. They are
regression tests, **not live product qualification**. Operate the daemon and
`bobe` CLI with a real provider for authentication, execution, persistence,
and restart evidence.

## Private protocol

`src/protocol.ts` defines the validated protocol. Each stdin/stdout line is
one UTF-8 JSON object, at most 16 MiB. Stdout contains protocol frames only.
Rust requests have positive safe integer IDs. Worker callback IDs are
negative. Requests run concurrently; responses may arrive out of order.
Malformed input, duplicate request IDs, and unknown callback IDs fail.

`fixtures/host-contract-v1.json` is the exact shared fixture for Rust contract
tests. `src/layout.ts` defines the runtime paths.

Requests are `{id, method, params}`. Replies are `{id, result}` or
`{id, error: {message}}`. Notifications have `{method, params}`.

| Method | Parameters | Result |
| --- | --- | --- |
| `initialize` | Protocol v1 owner paths, profiles, routes, exported tool schemas, capability instructions, MCP servers, enabled tools, execution limits and all workload budgets | `{protocol_version:1,harness:"pi",version:"1.0.3"}` |
| `chat` | `{session_key?,text,channel:"text"\|"voice",context,memory_context}` | `{}` after settled success |
| `background` | `{job_id,title,task,context,memory_context,attempt,route}` | `{session_id,output}` after the task and its resources settle |
| `auxiliary` | `{class:"goals"\|"small"\|"consolidate",job:{job_id,kind,instructions,input},context}` | `{job_id,output,error}`; failed output is `null`, never a fabricated object |
| `probe` | Optional `{route}`; defaults to active chat | `{model_execution,streaming,tool_execution,error}`; finite echo-tool check |
| `context_status` | Optional `{session_key}` | Channel pi session ID, context estimate, token/cache counters, tool totals and compaction count |
| `compact` | Optional `{session_key,keep_recent_tokens}` | Native pi summary, retained-entry ID and before/after context estimates |
| `abort` | `{request_id}` | `{}` only after the operation settles |
| `steer` | `{request_id,text,follow_up}` | `{queued:true}` for an active correlated run; native pi applies guidance at a tool boundary or after the current task |
| `reload` | Optional `{mcp,skill_directories}` | `{}` |
| `mcp_status` | `{}` | `[{name,connected,status,error}]` |
| `mcp_tools` | `{server_name}` | `[{name,description}]`, with excluded tools removed |
| `auth_status` | `{profile}` | `{is_authenticated,auth_type,host,login,status_message}` |
| `models` | `{profile}` | `[{id,name,vision,context_window,default_reasoning_effort,supported_reasoning_efforts}]` |
| `auth_login` | `{profile}` | Authentication status after the native OAuth flow |
| `auth_submit` | `{profile,value}` | `{}` |
| `auth_cancel` | `{profile}` | `{}` after login settles |
| `auth_logout` | `{profile}` | `{}` after any login settles and pi removes the authority-scoped OAuth credential |
| `shutdown` | `{}` | `{}`, then process exit |

Routes use pi reasoning values: `off`, `minimal`, `low`, `medium`, `high`,
`xhigh`, or `max`; `null` selects the worker default.

Execution notifications are
`{method:"event",params:{request_id,delta}}`. Delta variants are
`message_delta {message_id,text}`, `message_complete
{message_id,content,output_tokens}`, `tool_start {id,name}`, `tool_complete
{id,name,success}`, `error {message}`, and `done`. Parent assistant text
streams live. IDs stay stable per assistant message. Thinking, child text,
and tool arguments are never sent as reply text. Failed or truncated text
remains partial. `done` is emitted only after pi and persistence settle.
Automatic provider retries are disabled; overflow replay is also blocked
after visible output.

The owner uses `bobe agent status --json` to inspect these counters and
`bobe agent compact` to request native pi compaction. Compaction holds the
daemon's shared admission permit; it cannot interrupt another active owner
turn. `--keep-recent-tokens` changes only that compaction's retained tail.

Prompt caching is enabled through pi's provider adapters. Cache warming is
disabled to avoid unsolicited inference. Stable behavior, tool definitions
and memory form the prefix. Current time and channel hints are appended with
the new user turn rather than rewriting the old prefix. A changed memory,
skill or tool definition invalidates its affected prefix.
Provider cache reads/writes are reported, not inferred from elapsed time.
Provider tool-call IDs stay in the private transcript; owner events use
opaque correlated IDs without provider reasoning signatures.

`tool_activity {request_id,name}` precedes parent tool execution.
Voice requests use short speech guidance and pi's low reasoning setting,
clamped to the selected model. Generic interactive extension dialogs fail
explicitly rather than holding an unattended stack. Important product questions
use the durable `bobe_owner_input` tool and the real conversation/native surfaces.

Authentication notifications are
`{method:"auth_event",params:{profile,phase}}`. Tagged phases are
`preparing`, `awaiting_user {url,code}`, `polling {url,code}`,
`awaiting_input {message,password}`, `completed`, `failed {message}`, and
`canceled`. Native select prompts list their option IDs in `message`;
`auth_submit.value` must be one of those IDs.

## Owner callbacks and cancellation

The worker calls `domain_tool` with `{request_id,name,arguments}` and the original Rust
JSON schema. Rust returns `{success:true,value}` or a failed outcome.
Failed `value.error`, `error`, and `message` strings are reported as tool
errors. They are not converted into successful tool output.

Rust binds each domain callback to its admitted chat request and cancellation
token and authenticated conversation channel. Channel chats, background jobs and their scoped descendants receive
domain tools; auxiliary and probe sessions do not. The worker keeps
callback waiters until Rust replies; it does not acknowledge cancellation
while owner effects remain uncertain. The daemon's bounded kill remains the
last resort. No tool call is automatically replayed.

The companion in each channel can call `background_job` with
`{request_id,name,arguments,source_key?}`. Actions are `start`, `list`, `get`,
and `cancel`. `start` needs a SHA-256 source identity derived from the pi session
and assistant-message entry plus native tool-call ID, since provider-local IDs
can repeat in later turns. Rust journals the task and returns immediately; it
does not run another model loop. Background sessions may inspect/cancel tasks
but cannot recursively start immediate jobs. Ordinary jobs support explicit owner resume. Hourly active-goal review
can reopen an interrupted goal task for effect inspection and safe continuation;
it does not replay a stored tool call.

`bobe_skill_create` returns after Rust's durable, validated publication and a
worker resource reload. The protocol handles the nested reload while the tool
is pending. If activation fails, Rust restores the prior file and reloads it.
Success confirms both publication and activation, with no second skill owner.

MCP uses pi's native extension and a Rust-backed stdio transport. Rust supplies
`{command,args,env,timeout_ms,excluded_tools}` for each named server.
Environment values are already resolved; the worker does not evaluate
shell expressions or load `.env` files. An aborted active MCP call closes
its owned process. The next turn reconnects through pi without replacing
the channel session. Reload reaches every open channel; shutdown closes all
session and MCP resources.

Shell, MCP, grep and find processes are daemon children with private process
groups. Worker death cancels and reaps them before the run reports settlement.
Cold-start initialization and shutdown retain ownership if their caller is
cancelled. This is not an OS sandbox: daemon `SIGKILL` outside a service manager,
intentional daemonization and hostile grandchildren are not containment claims.

`agent.execution` bounds real inference, tools, processes and live channel
objects; [Admission, runs and resources](../docs/architecture.md#admission-runs-and-resources)
lists the Rust defaults.
Model permits are not held while a parent waits for children. Workload input
limits use native pi token estimates; output limits reach provider `maxTokens`.
Runtime task budgets select timeout and turn limits below owner ceilings.
Idle channel/MCP objects retire after the idle timeout or to satisfy cache
capacity; the exact channel pointer and transcript remain durable.
Rust supplies these policy values in every initialization. Missing limits or
workload budgets fail validation; the worker does not carry fallback policy
defaults. The shared host fixture checks Rust serialization against the worker
schema.

The `bobe_schedule` and `bobe_owner_input` tools keep their state in Rust.
[Durable jobs, schedules and important questions](../docs/architecture.md#durable-jobs-schedules-and-important-questions)
describes their behavior, and
[Initiative and delivery](../docs/architecture.md#initiative-and-delivery)
describes commitment and presentation receipts.

## Persistence and configuration

Owner state lives only below `data_root/agent/`:

- `session.json` contains `{version:1,session_id,file}` for the owner channel.
- `sessions/` contains the owner's pi transcript and channel/task directories.
- `sessions/channels/<channel-uuid>/session.json` selects each enrolled
  communication channel's exact transcript.
- `sessions/children/` contains scoped child transcripts with pi's
  `parentSession` link to their parent transcript.
- `sessions/jobs/<job-uuid>/session.json` points to that durable job's exact pi
  transcript. Resume opens it; it never replaces a channel conversation.
- `auth/<sha256(profile-provider-id)>.json` stores native OAuth. The provider
  ID is `bobe-profile:<profile-id>`.
- `auth/device-id` holds the stable SDK installation identifier.
- `runtime-files/` and `mcp.log` hold authority-local runtime artifacts.

All worker-owned credentials and the OAuth installation identifier are
inside `auth/`. Privacy purge can preserve only this directory; transcript
backups can exclude exactly this directory.

The exact pointer is opened with `SessionManager.open`. First use creates
a pi session and durably publishes its header and pointer before accepting
turns. Invalid entries, a missing transcript, and an orphan transcript
without a pointer require owner recovery. The worker never selects the
most recent session or replaces damaged history.

The nil UUID selects the existing owner pointer. Rust derives other channel
UUIDs from authenticated endpoint IDs; reconnect and credential rotation do
not create new channels. Every session receives the same authority workspace,
memory, skills and provider credentials. `bobe_history_recall` obtains bounded
excerpts from another channel only when needed.

Changing a provider or route restarts the supervised worker with new
initialization data and the same channel pointers. Tool, skill, and MCP refreshes do
not fork conversation history. Skill-only reload is safe inside a turn.
MCP configuration reload requires idle execution. `enabled_tools` is a
restart-bound capability selection shared by every full session. Disabled native
tools are explicitly excluded even when resumed history once declared them.

API keys arrive only in the private initialization request and stay in
memory. OAuth files are owner-scoped and private. No global pi auth,
settings, prompts, extensions, or project `.pi` resources are loaded.
The entry point removes unrelated ambient environment variables. The Rust
launcher must also remove `NODE_OPTIONS` before starting Node, since Node
processes that variable before the entry point can run.

## Models and bounded work

Rust supplies chat, goal-work, memory-consolidation and optional `small`
routes. Small decisions use `small` when configured, otherwise the goals route.
When `small` is configured, both manual and automatic compaction use pi's native
compactor with that model, retaining the same channel pointer. Without it,
native compaction uses the session model. Do not select a model merely because
its name sounds small; model access, vision and context limits come from the
provider catalog and explicit owner configuration.

OpenAI uses pi's OpenAI API and native subscription OAuth implementation.
Those access methods remain separate and account-specific. Anthropic uses its
native API-key implementation only. BoBe rejects Claude subscription OAuth:
[Anthropic's published policy](https://code.claude.com/docs/en/legal-and-compliance#authentication-and-credential-use)
does not permit third-party apps to offer Claude.ai login or intermediate
consumer subscription credentials. The existence of an upstream OAuth adapter
does not authorize its use in BoBe. GitHub Copilot is a pi
inference provider. DeepSeek and OpenRouter use pi's native providers.
Rail Relay is an external token-auth inference endpoint.

Profiles marked `local_runtime: "ollama"` read `/api/tags` and `/api/show`
for installed model IDs, vision capability and context metadata. Execution
still uses pi's OpenAI-compatible transport; this is catalog discovery, not a
separate inference loop.

Generic providers discover actual IDs from `GET <base_url>/models` and
execute through pi's native transports. Relay accepts an origin or a
`/v1` base URL. It reads both `/v1/models` views: the Bearer view without
Anthropic headers, and the Messages view with `x-api-key` and
`anthropic-version`. Their union is used; neither filtered view is treated
as universal. A failed view fails discovery instead of returning an
incomplete success. Duplicate model IDs retain the Bearer-view entry, so
route IDs remain stable.

Relay entries must specify `api: "messages" | "responses" | "chat"`. These
select pi's native Anthropic Messages, OpenAI Responses, or OpenAI Chat
Completions transport. Messages uses the root base URL so the SDK adds
`/v1/messages` only once. The other transports use the `/v1` base.

Catalog entries can supply `name`, `vision`, `context_window`,
`context_length`, `max_input_tokens`, `max_prompt_tokens`, and
`max_output_tokens`. The context bound is the lowest positive input or
context limit. Zero, negative, or missing bounds do not override a positive
limit. Unknown context metadata is reported as `null`.
Relay `capabilities.image_input`, `reasoning` and `effort` retain the actual
provider's vision and thinking-level support. Catalog refresh and selection
are serialized per profile so concurrent owner inspection cannot clear the
model selected by an active turn.
For execution only, an ID-only generic catalog uses conservative
limits of 8,192 context tokens and 2,048 output tokens, with vision and
reasoning disabled.
An unauthenticated local profile must be explicitly marked `offline`.
Non-loopback HTTP endpoints are rejected.

Auxiliary jobs return the requested JSON value through `bobe_submit_result`,
or a final `{"output": <value>}` envelope. A summary may be a string; activation
and consolidation return objects. Goals and consolidation use the
daemon's 3- and 15-minute deadlines; probes are bounded to 90 seconds.
A probe must execute and stream a real echo tool round.

The `subagent` tool follows pi's official MIT-licensed
[`examples/extensions/subagent`](https://github.com/earendil-works/pi/tree/main/packages/coding-agent/examples/extensions/subagent)
execution semantics, using in-process isolated SDK sessions rather than
spawning another CLI. Its input is `{mode:"parallel"|"chain",tasks:[...]}`:
a one-item list runs one child. This avoids conflicting optional input shapes
when providers generate arguments under strict JSON schemas.
Each child inherits native file/shell/search tools, domain tools, MCP,
skills, and the selected model. Context is explicitly scoped. Parallel
lists admit at most eight tasks and run four at a time. A turn can create
at most sixteen children with two delegation levels. Children are bounded
to ten minutes and 32 turns, and settle with their owner.

Child transcripts are durable audit records, not resumable background jobs.
Child execution is still scoped and awaited. Each channel pointer selects its
conversation; child transcripts never replace it.

`background_task` queues work that outlives the current reply. Rust admits and
budgets these jobs;
[Durable jobs, schedules and important questions](../docs/architecture.md#durable-jobs-schedules-and-important-questions)
gives the limits. Their nested scoped children keep the child budgets above.
Background execution uses the same `Sessions` implementation, not pi-durable or
a second agent loop. Results return through Rust admission to a communication
channel.
Goal work runs independently of app availability; the daemon separately uses
the small route to offer, defer or suppress a goal update. Interruption records
uncertainty, so resumption must inspect effects rather than repeat them blindly.
