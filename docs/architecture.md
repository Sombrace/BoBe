# Architecture

[PRODUCT.md](../PRODUCT.md) says what BoBe is; this page says which part owns
what. Source and tests are authoritative.

| Path | Owns |
|---|---|
| `BoBeService/` | The Rust daemon and `bobe` CLI: identity, conversations, memory, goals, jobs, schedules, credentials, policy, voice turns, BodyLink |
| `BoBeAgent/` | The private Node worker running the pi harness: model and tool loop, compaction, skills, subagents |
| `BoBeMacUI/` | The Mac app: presentation, screen reading, microphone, local speech recognition, playback |
| `BoBeiOS/` | The iPhone app: conversation, goals, memory, hold-to-talk |
| `PairingCore/`, `PairingFFI/` | OPAQUE pairing shared by the daemon and the Apple apps |
| `esp32-sdk/` | Firmware for physical bodies |

```text
Mac app (managed-local) ──launches──┐
Mac app (attached or remote) ──REST / SSE / voice WebSocket──┐
iPhone ──HTTPS──┐                   │                        │
bobe CLI ───────┴──────────────► bobe daemon (Rust, SQLite, files)
ESP32 bodies ──BodyLink mTLS──►     ├── Node worker (pi), started on first use
                                    ├── shell, rg, fd and MCP processes
                                    └── optional speech engines
```

The daemon is the only authority; everything else is its client. Swift never
starts pi, and the CLI never runs inference.

## Lifecycle

- **Ownership.** A managed-local Mac app starts the bundled daemon and alone
  may stop or restart it; the daemon exits when that app is gone. Otherwise
  (attached, remote, launchd, systemd) whoever started the daemon owns it.
- **Startup** takes one data root and its process-lifetime locks, finishes or
  rolls back an interrupted restore, loads the configuration, opens and
  upgrades the database, then starts services (`bootstrap/`).
- **Readiness** is `ready`, `initializing` (setup unfinished) or `not_ready`
  (database unavailable), computed once in `api/readiness.rs`. The Mac and
  iPhone attach while a daemon is initializing; the iPhone waits for `ready`
  before sending. Readiness does not include provider health.
- **Shutdown** stops admission, settles active runs and their processes, then
  closes the worker and the database. Unstarted messages stay queued.
- **Schema.** `migrations/schema.sql` creates a database, and
  `migrations/upgrade-<from>-to-<to>.sql` steps upgrade an older one at start,
  one transaction per step (`bootstrap/database.rs`). A change to `schema.sql`
  needs a new step and a test that upgrades the previous version's fixture.

## Worker

Rust starts one Node process on first use and exchanges JSON lines over stdio
([protocol](../BoBeAgent/README.md#protocol)). Rust sends every policy value:
paths, providers, routes, tools, limits and budgets. Shell, search and MCP
processes are daemon children in their own process groups; if the worker dies,
Rust reaps them before it ends the run. This is process ownership, not a
sandbox.

## Conversations and behavior

- Each authenticated endpoint is a channel with its own history and pi
  session. All channels share the workspace, memory, goals, skills and
  credentials; `bobe_history_recall` reaches another channel's history.
- The system prompt is the packaged `BoBeAgent/assets/chat.md`, the only
  behavior text, plus the enabled souls and profiles, memory, capabilities and
  learned skills.
- Rust sends events as a marker plus JSON (`[bobe.goal_work]`,
  `[bobe.proactive]` and others), written only by `event_block`; the worker
  adds `[bobe.turn_context]`. A block counts only as a whole message or task,
  or inside the turn context. Text from the model or stored documents is
  escaped before it reaches a prompt; the owner's own messages are not.

## Durable work and initiative

- Each channel runs one turn at a time from a durable queue. Human input
  preempts deferrable proactive work; explicit guidance can steer a running
  turn.
- `agent_jobs` and `agent_schedules` hold background and planned work. Enqueue
  and next-due advancement share one transaction, and missed intervals
  coalesce. Interrupted work waits for an explicit resume.
- `bobe_owner_input` saves a question with its exact action and settles the
  run. An answer given after the question was shown releases it once.
- Active goals are reviewed hourly; three reviews in a row that change nothing
  rest a goal until its document changes. A small-model decision offers,
  defers or suppresses each update.
- Screen text (from a Mac offering `screen.read`, every two minutes) and
  check-ins go to one small-model evaluator, the only owner of "should BoBe
  speak now". The main agent then writes the message.
- A stored reply is committed, and presented only on a receipt: visible chat,
  CLI output or finished playback. Saved updates are shown again without new
  inference.

## Trust and networking

- The daemon listens on loopback by default. Remote access is explicit and
  always TLS, with a certificate the daemon renews itself. The owner token is
  accepted only from loopback.
- Pairing is a six-digit OPAQUE exchange bound to the daemon's TLS key
  ([device pairing](device-pairing-crypto.md)). A Mac gets every scope and a
  phone a reduced set (`api/scope.rs`); each credential is revocable and only
  its verifier is stored.
- Tools run with the daemon account's permissions. Presenting a conversation
  never gives the daemon a surface's files, shell or screen.

## Persistence

SQLite (`migrations/schema.sql`) holds conversations, queues, jobs,
schedules, questions, cooldowns, endpoints, souls and profiles; memory, goals,
skills and the workspace are files ([data layout](cli.md#configuration)).
Backups pause the worker and exclude sign-ins. Restore verifies, swaps whole
directories with rollback and raises the authority epoch. A privacy purge
removes personal data and keeps credentials, installed runtimes and backups.

## Voice and bodies

The Mac recognizes speech locally; the daemon owns the turn and synthesizes
with Pocket TTS, streamed as Opus over `bobe.voice.v2` ([speech](speech.md)).
ESP32 bodies connect over mTLS BodyLink with owner-granted capabilities, and
one at a time holds the voice lease ([ESP32 SDK](../esp32-sdk/README.md)).

## Mac app

The app only presents; it never changes authority or admission.
`BoBeMacUI/BoBe/Design/` is the one design layer for the overlay, Settings,
setup and the iPhone, and `SettingsStore` is the only cache of daemon settings.
