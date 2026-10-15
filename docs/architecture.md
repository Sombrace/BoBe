# Architecture

[PRODUCT.md](../PRODUCT.md) defines what BoBe is. This page says which part owns
what. Source and tests are authoritative.

## Components

| Path | Owns |
|---|---|
| `BoBeService/` | The Rust daemon and the `bobe` CLI: identity, conversations, memory, goals, jobs, schedules, credentials, policy, voice turns, BodyLink |
| `BoBeAgent/` | The private Node worker that runs the pi harness: model and tool loop, compaction, skills, subagents |
| `BoBeMacUI/` | The Mac app: presentation, screen reading, microphone, local speech recognition, playback |
| `BoBeiOS/` | The iPhone app: conversation, goals, memory, hold-to-talk |
| `PairingCore/`, `PairingFFI/` | OPAQUE pairing, shared by the daemon and the Apple apps |
| `esp32-sdk/` | Firmware components and examples for physical bodies |

```text
Mac app (managed-local) ──launches──┐
Mac app (attached or remote) ──REST / SSE / voice WebSocket──┐
iPhone ──HTTPS──┐                   │                        │
bobe CLI ───────┴──────────────► bobe daemon (Rust, SQLite, files)
ESP32 bodies ──BodyLink mTLS──►     ├── Node worker (pi), started on first use
                                    ├── shell, rg, fd and MCP processes
                                    └── optional speech engines
```

The daemon is the only authority. The worker, the apps, the CLI and the bodies
are clients of it. Swift never starts pi, and the CLI never runs inference.

## Lifecycle

- **Managed-local:** the Mac app starts the bundled daemon, enrolls itself
  through a one-use secret on stdin, and alone may stop or restart it. The
  daemon exits within about a second once that app is gone.
- **Attached-local, remote, launchd, systemd:** whoever started the daemon owns
  its lifetime. A Mac connects and disconnects without stopping it.
- **Startup:** one data root (`DataPaths`), process-lifetime authority and
  database locks, then configuration, schema and services (`bootstrap/`).
  Readiness reflects the database and initialization, not provider health.
- **Shutdown:** stop admission, leave unstarted messages queued, settle active
  runs and their processes, stop listeners, close the worker, close the
  database.
- **Schema:** there are no migrations. The daemon refuses a database whose
  schema version differs from its own; changing `schema.sql` raises that
  version. The one exception deletes 0.2.0 data at start so those installs run
  setup again (`bootstrap/release_0_2.rs`).

## Worker

Rust starts one Node process on the first agent, catalog or auth request and
talks to it with JSON lines on stdin and stdout. Rust sends every policy value
(paths, providers, routes, tools, execution limits and task budgets) at
initialization or with each request. The protocol is defined in `BoBeAgent/src/protocol.ts` and checked
against Rust with `BoBeAgent/fixtures/host-contract-v1.json`.

Shell, search and MCP processes run as daemon children in their own process
groups, under one capacity limit. If the worker dies, Rust cancels and reaps
them before reporting the run's end. This is process ownership, not a sandbox.

## Channels and context

Each authenticated endpoint is a channel with its own history and pi session.
Typed and spoken input from one endpoint share its channel. All channels share
the workspace, memory, goals, skills and credentials. Another channel's
history is reached with `bobe_history_recall`, never by copying transcripts.

Session pointers live under `agent/`: the owner channel in
`agent/session.json`, others in `agent/sessions/channels/<id>/` and jobs in
`agent/sessions/jobs/<id>/`. The worker opens the exact pointer, so restarts,
model changes, reloads and compaction keep the same conversation.

Companion behavior is `BoBeAgent/assets/chat.md`, installed as
`skills/chat/SKILL.md` in the data root. An owner's edited copy is kept.

## Admission and durable work

- Each channel runs one conversational turn at a time; channels run in
  parallel under the limits in `BoBeService/src/config/agent.rs`.
- Incoming messages enter a durable per-channel queue. Human input preempts
  deferrable proactive work after the previous run has settled.
- Explicit guidance steers or follows an active run; an ordinary message is
  its own turn.
- `agent_jobs` holds background jobs; `agent_schedules` holds one-off and
  repeating work. Enqueue and next-due advancement share one transaction, and
  missed intervals coalesce into one run.
- Interrupted work is not replayed. Ordinary jobs wait for an explicit resume;
  a goal review may reconsider an interrupted goal task after checking its
  effects.
- `bobe_owner_input` saves a question with its exact action and continuation
  and settles the run. One answer releases one continuation. An answer binds
  only to a question already presented in that channel.

## Initiative

- The daemon asks a connected Mac that offers `screen.read` for screen text
  every two minutes. The Mac reads its display with Vision, discards the
  image and returns the text with the front app and window title; the small
  model turns it into one sentence of context.
- Active goals are reviewed hourly, with or without an open app. Three
  unchanged reviews in a row rest a goal until its document changes. The
  starter goal is created paused.
- A small-model decision offers, defers or suppresses each goal update.
  Presence and cooldowns limit speaking, not quiet work.
- A stored reply is *committed*; it is *presented* only on a receipt from
  visible chat, CLI output, finished playback or a body's playback drain.
  Saved updates are shown again without new inference.

Body requests such as `screen.read` travel as `body_request` events on the
asking connection's SSE stream and are answered once at
`POST /body/requests/{id}`. They are never replayed after a reconnect.

## Trust and networking

- The daemon listens on loopback by default. Remote access is explicit
  (`bobe config remote`) and always uses TLS; the daemon creates and renews
  its own certificate.
- Pairing uses a six-digit OPAQUE exchange bound to the daemon's TLS key
  ([device-pairing-crypto.md](device-pairing-crypto.md)). Each Mac or phone
  gets its own revocable credential; only its verifier is stored.
- A Mac pairs as `mac_app.v1` with every scope. A phone pairs as `phone.v1`
  with conversation, events, voice, goals, memory and settings-read scopes
  only. Scopes are in `BoBeService/src/api/scope.rs`.
- The signed Mac app keeps its credential in a Data Protection Keychain
  group. A build without a provisioned identity keeps it in memory only, so
  attached and remote pairings end when the app quits.
- A loopback peer may use plain HTTP; network peers always need TLS. The
  owner token is accepted only from loopback.
- Tools run with the daemon account's permissions. Presenting a conversation
  never gives the daemon a surface's files, shell or screen.

## Providers

Four routes pick models for workloads: `chat`, `goals`, `small` and
`consolidate`. Profiles remember their choices; provider, credential and
route changes apply on restart. API keys live in the daemon's headless secret
store and pi sign-ins in `agent/auth/`. Anthropic is API-key-only.

`bobe local-runtime install` can download and start Ollama, but a later daemon
start does not restart it.

## Persistence

| State | Where |
|---|---|
| Conversations, queues, jobs, schedules, questions, cooldowns, endpoints | SQLite (`BoBeService/migrations/schema.sql`) |
| Memory and goals | `memory.md`, `goals/` |
| Skills | `skills/` |
| Working files | `workspace/` |
| pi sessions and sign-ins | `agent/`, `agent/auth/` |
| API keys | `headless-secrets.json` (mode 0600) |
| Mac endpoint credential | Keychain, or memory in an unsigned build |
| Surface preferences | UserDefaults |

Backups pause the worker and include everything except sign-in state.
Restore verifies first, swaps whole directories with rollback and raises the
authority epoch. A privacy purge removes personal data and keeps credentials,
installed runtimes and backups.

## Voice

The Mac captures audio and normally recognizes speech locally; the daemon
owns the turn, the reply sentences and Pocket TTS synthesis, streamed as Opus
over `bobe.voice.v2`. A turn stays open until the surface reports that
playback finished. Talking over BoBe is judged on recognized words, and the
next turn is told which sentences were heard.
Engines, models and licenses are in [speech.md](speech.md).

## Bodies

ESP32 bodies connect over a separate mTLS BodyLink listener with enrolled
identities and owner-granted capabilities. Many bodies may connect; one holds
the voice lease. Each body has its own channel. Queued body actions fail on
disconnect instead of running elsewhere. See the [ESP32 SDK](../esp32-sdk/README.md).

## Mac app

The app only presents; it never changes authority or admission.
`BoBeMacUI/BoBe/Design/` is the single design layer for the overlay, Settings,
setup and the iPhone: controls, typography and themes, `BobeMotion` for
animation curves (Reduce Motion is read in one place, `AccessibilityRuntime`),
`BobeMaterial` for surfaces and `Design/Character/` for the one BoBe face.
`SettingsStore` is the app's only cache of daemon settings. The overlay is two panels sharing
`OverlayModel`: the avatar panel never takes the keyboard, and the
conversation panel takes it only while the owner writes. Only the owner's
drag moves the avatar.
