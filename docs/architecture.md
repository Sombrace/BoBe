# BoBe implementation architecture

Source and tests are authoritative. [PRODUCT.md](../PRODUCT.md) defines product
boundaries, and the guides describe supported workflows and current limitations.
This document describes implemented ownership.

## Source layout

| Path | Responsibility |
|---|---|
| `BoBeService/` | Rust daemon, owner CLI and durable authority |
| `BoBeAgent/` | Private Node/pi worker |
| `BoBeMacUI/` | Native Mac presentation and local speech |
| `BoBeiOS/` | Native iPhone surface, pairing and local speech input |
| `esp32-sdk/` | ESP32 client libraries, board examples and firmware tools |
| `PairingCore/`, `PairingFFI/` | Shared pairing implementation and native interface |

The SDK builds separately from the daemon and app. BodyLink changes can update
Rust, Swift and device code in one commit; the wire check uses `esp32-sdk/` by
default.

## Process ownership

```text
Managed-local Mac                    Attached-local or remote Mac
BoBe.app / BackendService            BoBe.app
  | launches and supervises            | REST / SSE / voice WebSocket
  v                                    |
bobe-daemon <--------------------------+
  Rust authority, Tokio tasks, SQLite and files
  |
  +-- starts one private Node process on first agent/auth/catalog use
  |     BoBeAgent/dist/main.js
  |       pi AgentSession objects: channels, jobs, subagents, finite operations
  |       provider HTTP, native model/tool loop, compaction and tool scheduling
  |
  +-- starts shell, rg, fd and stdio MCP children requested by Node
  |     Rust owns their process groups, input/output, cancellation and exit
  |
  +-- loads optional speech engines inside Rust; no second companion process
  +-- optional local-model installer can start its downloaded Ollama

Standalone host
launchd / systemd / operator -> bobe serve -> the same Rust/Node boundary

ESP32 bodies -> authenticated BodyLink -> the same Rust authority
```

An `AgentSession` is not an OS process or thread. One Node event loop hosts
separate logical contexts. Rust never calls a second provider-specific agent
loop, and Swift never starts pi directly. The CLI is an HTTP/SSE client of the
selected daemon, not another inference authority.

| Component | Owner and responsibility |
|---|---|
| Managed-local daemon | `BackendService` starts the bundled daemon, enrolls the Mac and waits for readiness. It alone owns app-driven stop/restart. |
| Attached/remote daemon | The external operator or service manager owns its lifetime. The Mac connects and disconnects without stopping it. |
| Node worker | `agent/host.rs` starts it lazily, sends private configuration and retains it through initialization, cancellation and shutdown. A failed worker is re-created on later use, not by a competing daemon loop. |
| Model/session execution | `BoBeAgent` embeds pinned pi coding-agent, agent-core and pi-ai. Pi executes reasoning, tools and compaction. |
| Tool processes | `agent/process.rs` launches them on worker request. Shell, search and MCP all use this owner and its aggregate capacity. |
| Durable work | Rust job/schedule/input repositories own admission, state transitions, next-due times and continuation identity. |
| Mac speech/presentation | Swift owns screen reading, microphone capture, local speech engines when selected, playback and UI state. |
| Physical body | Firmware owns local I/O, buffering, physical privacy controls and bounded safety. It holds no model credentials or companion memory. |
| Local inference server | Separate from the harness. [Providers and local inference](#providers-and-local-inference) describes Ollama startup ownership. |

## Startup and shutdown

`bobe serve` resolves one canonical `DataPaths`, acquires process-lifetime
authority/database locks, loads configuration and credentials, applies the
SQLite schema and constructs services in `bootstrap`. There are no schema
migrations: an existing database must carry exactly the schema version this
daemon uses, or startup refuses it untouched and a new data root is needed. Any
change to `schema.sql` raises that version. It recovers job and
message journals before serving. Optional speech-model loading runs separately;
installed models are not advertised as ready until their engine checks pass.

Rust background tasks handle administration, triggers, durable work,
consolidation, retention and the optional BodyLink listener. Companion
schedulers wait for initialization readiness. HTTP readiness reflects database
and initialization state, not continuous provider availability; a provider
outage must not cause a service restart loop.

The first execution, model catalog, authentication or agent-status request can
start Node. Rust sends owner paths, providers, routes, tool definitions,
execution ceilings and workload budgets. Node imports pi only after selecting
the authority's private agent directory. Secrets and unrelated ambient
configuration are not loaded from a global pi or provider CLI home.

On daemon shutdown:

1. Stop accepting new input and stop queued admission.
2. Leave unstarted surface messages durably queued; cancel active work and
   settle its tool/process and partial-output state.
3. Stop listeners and background tasks, then close the worker terminally.
4. Drain message bookkeeping and close the database.

`Host.close` prevents a new worker after final shutdown. Backup/purge use a
separate restartable quiescence operation. Both retain a tracked connection if
their caller is cancelled.

Node and each tool launch have owned Unix process groups. Worker death causes
Rust to cancel and reap its tool children before exposing terminal run state.
Graceful stop is bounded, with targeted TERM/KILL escalation. This is not an
OS sandbox or a guarantee against deliberate daemonization. Daemon `SIGKILL`
outside a service manager is a distinct containment limit.

A daemon started by the Mac app does not outlive it. With
`--managed-bootstrap-stdin` the daemon remembers its parent process and shuts
down within about a second of that parent being gone, so a crash or force
quit of the app cannot leave a hidden daemon running. Daemons started any
other way keep their own lifecycle.

Managed-local Swift publishes process ownership immediately after launch,
before bootstrap-pipe writes can fail. Stop calls are serialized, retain the
child until exit, and allow the daemon's shutdown budget before escalation.
Startup and enrollment completion are fenced against a stopped or changed
endpoint.

## Private Rust/TypeScript protocol

The worker speaks correlated JSON lines over private stdin/stdout, not a public
HTTP server. Stdout is protocol-only; redacted diagnostics use stderr.
Frames are capped at 16 MiB. Rust request IDs are increasing positive integers;
worker callback IDs are decreasing negative integers. Pending requests,
callbacks, stream subscribers and output queues are bounded.

Concurrent RPC handling allows a model/tool run to receive cancellation,
authentication input and resource reloads. A domain tool may call back into
Rust while Rust requests skill activation from the same worker; a global
request lock would deadlock this path.

Rust owns execution-policy values. The initialization schema requires all
ceilings and workload budgets; TypeScript does not invent a second set of
defaults. The shared credential-free host fixture is checked against Rust
defaults and the worker schema. Full method and event shapes live in
[BoBeAgent/README.md](../BoBeAgent/README.md) and `src/protocol.ts`.

## Identity, channels and context

One person has one authority, memory, goal system, skill store and workspace.
Provider profiles select access, not identities. Each authenticated
communication channel has its own displayed history and persistent pi context.
Typed and spoken input from the same endpoint share that channel.

The owner pointer is `agent/session.json`; endpoint pointers are
`agent/sessions/channels/<channel-id>/session.json`. Channel IDs derive from
enrolled endpoint identity, not a socket, credential generation or untrusted
selector. Job pointers are `agent/sessions/jobs/<job-id>/session.json`.
Scoped children have linked private transcripts.

The worker opens the exact pointer, never the latest file. Restart, model
changes, skill/MCP reloads and native compaction retain logical identity.
Idle live session/MCP objects can retire without deleting durable state.
Inactivity can close a display group but does not reset pi context.
Use `bobe_history_recall` for bounded, dated cross-channel evidence rather
than automatic transcript copying.

Behavior comes from `BoBeAgent/assets/chat.md`, embedded by Rust and installed
as the authority's `skills/chat/SKILL.md`. Existing owner content is preserved;
a newer shipped `chat.md` does not replace it. Memory and stable
tool/skill declarations form the cache prefix. Time and turn-specific hints
accompany new input. Pi owns compaction; an optional small-model route calls
pi's native compactor rather than a second context engine.

## Admission, runs and resources

Each channel admits one conversational run at a time. Different channels can
run concurrently. Accepted surface messages enter a bounded durable FIFO
queue. Human input preempts deferrable proactive/delivery work, but the old
permit remains owned until cancellation and persistence settle.

`RunRegistry` correlates user, proactive, background, delivery and voice runs.
`/runs` and `bobe runs` expose channel controls. Explicit guidance uses native
pi steering/follow-up; an ordinary new message remains a separate turn.
Indicators, text/tool events and snapshots are channel-scoped.

`agent.execution` defaults are four provider calls with one interactive slot
reserved, eight tool calls, 64 tool child processes, eight live channels, and 900 seconds
to idle retirement. Task ceilings default to 1,800 seconds and 128 turns.
Job and subagent requests may select smaller budgets. A provider permit is held
only during inference, never while a parent waits for children.

Workload prompt limits use native pi token estimates, including system/tool
context; these are not exact tokenizer or billing guarantees. Output limits
reach provider `maxTokens`. Ordinary file mutation queues do not make arbitrary
shell/Git side effects conflict-free; the agent must coordinate shared work.

## Durable jobs, schedules and important questions

`agent_jobs` owns route snapshots, attempts, outcomes and delivery IDs. At most
16 jobs are queued, running or waiting; two execute concurrently. Default job
budgets are 30 minutes and 64 turns. Model-origin creation IDs include the pi
session, assistant entry and tool call, because provider-local call IDs may
repeat. Background execution may inspect/cancel work but cannot recursively
start immediate durable jobs.

Interrupted effects are uncertain. Ordinary jobs need explicit resume; active
goals may reconsider interrupted work with retained context and effect
inspection. Claim/read and delivery transitions are fenced to the attempt.
Submission, execution and delivery participate in personal-data exclusion.

`agent_schedules` stores once/interval intents. Job enqueue and next-due
advancement share an immediate SQLite transaction. Missed intervals coalesce,
and a schedule does not overlap its own active/waiting occurrence. Pausing stops
future and queued work; running work has a separate stop control.
`runtime_cadences` preserves the hourly goal-review clock with atomic work
admission. Content-free tombstones prevent old creation requests from
recreating retired work.

Goal review is not endless. A goal ends when the companion or the owner sets
it to completed, and stops when it is paused because it cannot be achieved or
needs the owner. Independently of the model, the daemon stops queueing reviews
for a goal once three completed reviews in a row have left its document
unchanged, and starts again when the document changes. The goal stays active
for conversation while it rests. The starter example goal is created paused,
so a fresh install spends nothing on it.

Rare necessary owner decisions use `bobe_owner_input`. Rust saves the exact
question, action, continuation, expiry and response identity. The originating
run settles rather than keeping a worker/tool stack waiting. Other tools in
the asking round are blocked. A saved answer releases one continuation after
origin settlement; decline, expiry and cancellation do not.

An implicit short answer binds at arrival to the last recently presented,
complete question in that channel. Its identity is retained through message
queueing; a later question cannot capture an earlier "yes". Native buttons and
`bobe input answer` send an explicit request/response identity. The model cannot
answer its own request. Routine errors and technical retries are not approvals.
Generic extension dialogs fail explicitly; they are not durable resumable
product questions.

## Initiative and delivery

Observation defaults to two minutes and goal review to one hour. Observation
adds bounded context from text read on the device; it is not the goal scheduler.
The daemon keeps the clock and asks a connected Mac that offers `screen.read`.
The Mac captures its primary display with ScreenCaptureKit, recognises the text
with the Vision framework and discards the image. Only the recognised text (at
most 6,000 characters), the frontmost application and its window title come
back, and the daemon sends them to the small-model route as a bounded
`screen_activity` job that returns a one-sentence description. Managed-local,
attached-local and remote Macs use this same path; the daemon contains no
screen capture and behaves the same on Linux and macOS. No picture is sent to
any model, so pictures, video and drawings without text are not described.
Active goals can make quiet progress without an open app. Pi chooses useful
work within guidance, records verified outcomes, or waits for missing facts.

A separate finite small-model decision can offer, defer or suppress a goal
update. Presence and cooldown constrain interruption, not quiet work.
Activation rechecks current channel admission and timing. Bodies do not speak
unsolicited without an appropriate voice interaction.

A complete stored response is `committed`, not necessarily `presented`.
Presentation receipts come from visible native chat, explicit question display,
CLI output, the surface's report that spoken playback finished, or BodyLink
playback drain. Producing audio or sending SSE alone does not prove that the
person heard/read it.
Snapshots include unpresented saved updates; redisplay does not rerun inference.
Uncertain generation remains interrupted rather than being replayed blindly.

SSE uses bounded per-client subscriptions, incarnation-scoped cursors and
snapshot recovery. Text/tool events stay within the authenticated channel;
runtime health is shared.

A connection names what its body can do in the `BoBe-Body-Actions` header;
the only action is `screen.read`. The daemon asks that body through a
`body_request` event on the same stream. The event carries no event id, so it
never moves the cursor and is never replayed after a reconnect. The body
answers once at `POST /body/requests/{id}`. Only the connection that was asked
can answer, a connection holds at most four unanswered requests, and a
disconnect, a revocation or the request's deadline settles it as failed.

## Deployment and trust

Managed-local launches the bundled `bobe-daemon`, Node and worker assets.
The Mac exchanges a one-use inherited-stdin bootstrap secret for a normal
endpoint credential. Attached-local pairs explicitly and is externally managed.
Remote attachment requires deliberate authenticated TLS.

When manual TLS files are absent, the daemon can create its private identity.
OPAQUE pairing binds authority, epoch, root origin, transport, nonces and TLS
SPKI. Endpoint credentials are verifier-only in SQLite; the Mac stores its
device credential in its non-synchronizing Data Protection Keychain group.
An app with no provisioned identity (no application identifier and no Keychain
groups in its entitlements: an unsigned or ad-hoc build) cannot use that group
and holds the credential in process memory only (`EndpointCredentialBacking`).
Managed-local needs nothing stored: each daemon start carries a new bootstrap
secret, and enrollment replaces the previous credential for the same endpoint.
An attached or remote pairing in such a build ends when the app quits. A build
that carries any signing identity must match the endpoint group exactly or
every credential operation fails. Release signing checks the group before an
app can ship.
Revocation closes matching live sessions. Pairing cryptography, ABI ownership
and release gates are in [device-pairing-crypto.md](device-pairing-crypto.md).

Each pairing carries a scope profile, chosen by the owner's invitation and
bound in the OPAQUE transcript. A Mac pairs as `mac_app.v1` and receives every
scope. A phone pairs with `bobe device pair --kind phone` as `phone.v1` and
receives only what a phone body needs: `conversation_read`,
`conversation_write`, `events_subscribe`, `voice_session`, `goals_read`,
`goals_write`, `memory_read`, `memory_write` and `settings_read`. A lost phone
therefore cannot reach provider, body or configuration administration, submit
body results, steer a run, manage jobs, schedules or tools, or purge personal
data. Reading jobs and schedules needs only `goals_read`; the purge route needs
`settings_write` and steering a run needs `provider_admin`, which the Mac and
the owner CLI hold. The endpoints table has no phone kind, so a paired phone is
stored as kind `other` with `"scope_profile":"phone.v1"` in its enrolled
capabilities, beside the microphone, playback and recognition it declared; a
phone may declare nothing else, at pairing or at `POST /v1/hello` with surface
kind `phone`. An endpoint keeps the kind it was enrolled with, so a pairing
that names an active endpoint of another kind is refused instead of replacing
it. An event stream whose credential lacks `capture_submit` is never sent body
requests, whatever actions it declares.

A phone can pair only with a networked daemon. Its pairing link (`bobe://pair`)
carries the code, the SHA-256 of the daemon's TLS public key, and every allowed
host with the daemon's port, preferred first, so the phone trusts its first
connection before it sends the code. `bobe device pair` renders the link as a
terminal QR code; [cli.md](cli.md#pairing-a-phone) has the format.

The owner's app makes the same invitation over HTTP. `POST /pairing/invitations`
takes `{"kind":"phone","label":"iPhone"}` (the label is optional) and needs
`body_admin`. It runs the admin socket's invitation code and returns the same
JSON as `bobe device pair --kind phone --json`: `code`, `expires_in_seconds`,
`kind`, `label`, `pin`, `origins` and `link`. Only phone invitations are offered
over HTTP; a Mac still pairs from the daemon's own computer. A daemon no phone
can reach answers `409` with error code `NOT_NETWORKED`, and the CLI exits 2
with the same message. `DELETE /endpoints/self` needs only an endpoint
credential: it revokes the caller's own endpoint and closes its live sessions,
as `bobe device revoke` does, and answers `204`. The endpoint id `self` is
reserved for it.

The iPhone app keeps its credential in the Keychain on that device only and
pins every connection to the key it paired with. App Transport Security cannot
accept a self-signed key, so the app turns it off and its own pin check
decides trust, for task-level challenges from streamed requests as well as
session-level ones; it connects only over `https`. A phone that unpairs
revokes its own endpoint and pairs next time with a new identity, because a
revoked endpoint cannot be reactivated. The
[iPhone guide](iphone.md#reconnection-and-speech) describes hold-to-talk speech
recognition.

The daemon is loopback-only by default. Remote configuration cannot create
network reachability. A remote authority also does not gain access to an
attached Mac's filesystem or shell merely because it presents the conversation.

A networked daemon serves its single port to two kinds of peer. Every peer may
use TLS; a loopback peer whose first byte is not a TLS handshake is served
plain HTTP instead, so a managed Mac keeps its local connection after the owner
lets other devices reach BoBe. Peers from the network always need TLS. While
the daemon is networked, the owner token is accepted only from a loopback
peer and only when the daemon listens on the network directly; behind a
reverse proxy on loopback it stays refused, because proxied requests also
arrive from loopback. Automatic certificates name the allowed hosts plus
`127.0.0.1`, `::1` and `localhost`, and carry the serverAuth extended key
usage that Apple platforms require of TLS server certificates.

On a managed-local Mac, Settings > Devices > iPhone access (or Pair iPhone
on a BoBe that only listens on this Mac) asks once, then sets `server.host` to
`0.0.0.0` and `server.allowed_hosts` to the Mac's `.local` name and its IPv4
addresses, local networks before tunnels, and restarts the managed daemon.
Turning it off restores `127.0.0.1` with no allowed hosts. The managed daemon
is started without `--host`, so its configuration decides where it listens.
How the phone reaches that network, including VPNs such as Tailscale, is the
owner's choice; BoBe only lists the addresses it has.
Host tools carry the authority OS account's permissions; enabled-tool selection
is not hostile multi-tenant isolation.

## Providers and local inference

The native Models panel separates connections from workload assignments. Editing
access never reassigns workloads. Model choices patch one route and preserve the
other routes and their reasoning effort. `GET /providers` reports both saved
and active profiles/routes; pending changes are not labeled as running.
Catalog and auth operations depend on that profile's active access, not on
unrelated pending model choices.

`POST /providers/{id}/probe` performs a finite model/stream/tool check against
the requested model without changing the companion's routes or transcript.
`POST /auth/{profile}/logout` cancels an unfinished login and uses pi's native
credential removal; the owner CLI exposes `bobe provider auth logout --profile`.
Removing an OAuth profile also signs it out. A profile
still referenced by saved or running routes cannot be removed.

Privacy displays each running workload's destination and the authority host.
Local inference means local to that host, not necessarily the attached Mac.

Four routes select chat, goals/background, small decisions/summaries and
consolidation models. Profiles remember choices. Provider/credential/route
changes are restart-bound. Catalog metadata, not a guessed model name,
determines capabilities. Optional GitHub Copilot inference uses pi.

API keys live in the authority headless secret store. Supported subscription
flows use pi credentials under `agent/auth/`, not a global CLI account.
Rail Relay takes an owner-pasted token. Anthropic is API-key-only because its
policy does not permit third-party Claude subscription login.

Local compatible endpoints need no cloud credential when configured without
one. The installer discovers/downloads Ollama, uses its native catalog and
model-install APIs, and may start the managed executable if no server is
healthy. Inference still uses pi's OpenAI-compatible provider.

An installer-started Ollama is not re-established by normal daemon startup.
This lifecycle gap, custom endpoint ownership and the choice of local
runtime are deliberately parked by the owner for a later decision. Do not
describe this path as qualified continuous local inference.

## Persistence

| State | Storage/owner |
|---|---|
| Identity, conversations, message/job/schedule/input journals, cooldowns, endpoint trust | Rust-owned SQLite |
| Memory and goal documents | `memory.md` and `goals/` in the authority root |
| Canonical and learned behavior | `skills/`; validated publication and live reload |
| Shared working files | `workspace/` |
| Model context, channel/job pointers and child transcripts | `agent/` |
| Provider OAuth | Private `agent/auth/` |
| Daemon/provider/MCP secrets | Private secret-store files, not GUI Keychain |
| Mac endpoint credential | Data Protection Keychain; process memory in a build without a provisioned identity |
| Surface preferences | UserDefaults |

Configuration resolves defaults, persisted TOML and supported environment
overrides. Rust owns canonical paths and authority/database locks. Ordinary
capture/check-in/theme/voice preferences can hot-apply; provider routes and
execution ceilings require restart.
Conversation append/finalization transactions reserve the SQLite writer before
reading mutable state. This matches job/schedule/input journals and avoids WAL
read-to-write upgrade failures when recovered input overlaps startup work.

Backup quiesces pi and includes managed data and transcripts, excluding OAuth
state. Restore verifies artifacts, replaces complete managed roots with
rollback, preserves current OAuth credentials and increments the authority
epoch. Privacy purge preserves credentials, installed runtimes and owner
backups, removes personal work/context, and retains content-free replay guards.
A backup restores only into a daemon with the same backup format and schema
version. Owner commands are in the [CLI guide](cli.md#updating). Release
signing and publication are in [releasing.md](releasing.md).

## Voice and physical bodies

Swift owns native microphone capture, resampling, local STT/VAD/EOU when
selected, and playback. Rust owns authoritative turns, tools, sentences,
persistence and optional server speech. A frozen `SpeechPlan` selects engines
per voice session. The socket speaks `bobe.voice.v2` only: the surface sends a
speech offer, transcripts by default, or credit-bounded 20 ms PCM frames to a
readiness-qualified daemon STT engine. Raw audio is not stored.
Daemon synthesis uses native Pocket TTS with an English pack, bounded streaming
PCM and a persistent Opus packetizer. See [Speech](speech.md) for the
runtime, pack provenance, language scope and licensing.
Tool-activity filler output is keyed to the authoritative run. Concurrent
Mac/body turns cannot redirect each other's filler audio, and ending one run
removes only its output binding.

A voice-socket turn stays open until the surface reports that playback
finished (`tts_playback_complete`), whether the daemon or the surface
synthesised the speech. The daemon waits for that report for the length of the
audio it synthesised plus 30 s, or 10 minutes when the surface synthesises. Generating or sending the audio does not end the turn,
so the reply can be interrupted for as long as it is audible and the Mac shows
"speaking" for exactly that long. The Mac measures playback on the player's own
clock: audio scheduled after a silent wait starts at the playhead, not at the
end of the previous buffer.

Reply text is cut into sentences before it is cleaned for speech
(`SentencePipeline`). A sentence is released once the next one begins, at a
line break, when the text stream pauses for 250 ms (so a line said before a
tool call is spoken at once), or when the reply ends. A number or a decimal
split across streamed tokens is therefore never mistaken for a sentence end.
Emphasis marks, links and fenced code are removed from each whole sentence;
headings, bullets and list numbers are removed only where a line starts, so a
number that is itself the answer is still spoken.

Speaking over BoBe is judged on words, not on sound alone. The Mac reports
overlap when the microphone stays above its level threshold while BoBe plays,
except in the first 350 ms after speech becomes audible while echo
cancellation settles, and lowers BoBe's volume while the overlap lasts. The daemon then decides from the
live transcript (`judge_barge_in`): a short acknowledgement such as "yeah",
"mm-hmm" or "got it" never stops the reply; "wait", "no", "actually" and similar
openers stop it on one word; anything else stops it at two words, or four
characters in scripts written without spaces; cancel phrases stop it at any
time. If the sound ends before enough words arrive, the overlap stays pending
for two seconds and a later partial or final transcript can still stop the
reply and start the owner's turn. If nothing confirms it, the volume returns
and BoBe keeps talking. The Mac mirrors the same word rule for locally
synthesised speech (`BargeInPolicy`). The acknowledgement and opener lists are
English; other languages fall back to the word and character counts.

The companion is told what was actually heard. Each overlap report carries the
playhead (`playhead_chunk`): the first reply chunk that had not finished
playing when the overlap began. The daemon keeps the sentences it sent for the
turn with the chunk at which each one ends, so when the turn is interrupted it
knows which sentences were played in full and which one was cut. The next turn
in that communication channel, spoken or typed, carries one bounded hint with
the heard sentences and the cut-off sentence (`interruption_hint`). The stored
reply and the thread keep the full generated text. The pending hint is held in
memory, so a daemon restart between the interruption and the next turn loses
it. Stopping speech from the keyboard or by typing a new message is not
reported with a playhead and produces no hint.

The recognizer's end-of-utterance signal still ends the owner's turn, with one
exception on the Mac: an utterance that trails off on a filler, conjunction,
article, preposition or comma (`TurnCompletion`) is held for 1.1 s, at most
twice, and joined with whatever the owner says next before it is sent. The
trailing-word list is English; other languages are held only on punctuation.

Physical bodies use a dedicated mTLS BodyLink listener and registry-bound
identities. Multiple bodies can connect, while one physical voice lease owns
active speech. Route/generation IDs, bounded buffers, credit and played-sample
acknowledgements prevent cross-endpoint output and stale playback.
The Swift body speech adapter is a fallback, not an agent or a required Linux
authority component.

BodyLink text/action input uses the same durable request service. Its history
belongs to that body channel, not the Mac channel. Queued device actions are
failed after disconnect/restart rather than silently run as desktop messages.
Sensors are bounded expiring observations and do not start model turns by
themselves. Physical behavior and acoustic gates are in
[physical-bobe.md](physical-bobe.md) and the [ESP32 SDK](../esp32-sdk/README.md).

## Mac surface

The Mac surface is presentation only; none of the following changes authority,
admission or wire contracts.

`BoBeMacUI/BoBe/Design/` is the shared design layer for the overlay, Settings
and setup. `BobeMotion` holds the springs; every animation goes through it so
Reduce Motion disables all of them in one place. `BobeMaterial` holds the warm
raised/well surfaces, icon tiles, the shimmer text and the working rim. Glow
colors appear only while BoBe is working.

`Design/Character/` draws one BoBe for every surface. `BobePose` is an
interpolable face, `BobeMood` maps runtime state to a pose and to ring/moon
behavior, and `BobeCharacter` adds blinking, breathing, gaze, squash and the
orbit. Faces change by spring
between poses, never by swapping views. A per-frame timeline runs only while
the moon orbits or the ring follows a voice level; blinking, glances and
breathing are timer-started springs. Theme palettes come from
`ThemeConfig.avatar`.

The overlay has four presentations: collapsed (avatar only), ambient (a
whisper bubble, status pill or notice), ask (the capsule with the current
exchange) and conversation (full history).

The avatar is the anchor. Its screen position changes only when the owner drags
it, and it is the only handle that moves BoBe. The drag moves the anchor
directly (`OverlayWindowManager.dragAvatar`), keeps the avatar on a screen, and
re-chooses the side the conversation opens on while the drag is in progress.

The overlay is two borderless panels that share one `OverlayModel`. The avatar
panel is exactly the avatar and its rim controls and can never take the
keyboard. The conversation panel holds the column (bubble, pill, notices, or
the capsule and thread), is ordered out whenever nothing is shown, and is sized
to its content plus the shadow margin. Its SwiftUI tree is laid out on a fixed
canvas and the panel is a viewport on it, so resizing the panel never moves
what is drawn. The column opens toward the side of the avatar with room: left
or right, upward or downward, with the capsule always next to the avatar.

Neither panel blocks what it does not show. The avatar panel takes the mouse
only while the pointer is over the avatar or its rim controls; the conversation
panel, while it shows a bubble or notice, only over those pieces. Each visible
piece reports its frame and `OverlayWindowManager` compares the pointer against
them, so global pointer monitoring runs while the overlay exists. While the
capsule is open the conversation panel keeps the mouse and the keyboard, and a
click on its shadow margin counts as a click outside.

The keyboard moves only on the owner's action. Opening the capsule makes the
conversation panel key and focuses the field; closing it hands the keyboard
back to the app that had it. Clicking anywhere outside BoBe folds the capsule
away unless a draft is being written; a reply from the last two minutes, or one
still arriving, stays as a bubble. Escape and the close control dismiss it
completely. Real pointer and key events are the test for this area: the
accessibility press action bypasses hit testing and key focus.

The thread holds the whole conversation with the current exchange in view;
scrolling up moves into earlier messages and opens the taller conversation
presentation. A reply longer than the view starts at the owner's question
rather than at its end. Clicking the avatar toggles the capsule, and the chat,
microphone and Settings controls ride the avatar's rim while the pointer is
near. Sending keeps the capsule open; an active run turns the send button into
guidance or stop. While BoBe thinks or uses a tool the avatar's rim glows, its
ring becomes a comet, and a badge names the kind of tool.

The menu bar item is a template drawing of BoBe with a short menu: show or
hide, Settings, a More submenu and Quit. Quit is scheduled on the run loop in
its default mode so that termination never starts inside a menu's tracking
loop, hides the overlay and menu bar item at once, and is bounded by a
process-wide deadline.

A message BoBe starts on its own is a reach-out, and the overlay gives it one
of three kinds. An answer follows the owner's own message. A report is the
result of a job or schedule; the overlay learns that from the run kind of the
run that carried the message (`delivery` or `background`). Everything else is a
thought. The wire does not say whether a thought came from screen context,
a check-in or a goal, so the overlay shows one label for all three. A thought
or report carries one-tap replies in its bubble: reply, ask for more (sent as
a normal owner message), or dismiss. A reach-out the owner never touched, or
one that arrived in quiet mode, is not lost when its bubble leaves: it waits,
the avatar shows a count, hovering the avatar brings the newest one back, and
opening the conversation clears the count. The waiting list lives in the Mac
process only and does not survive an app restart.

Settings is a source list on the ground with one framed stage. Setup is the
same frame with BoBe beside the steps; the orbit ring is setup progress. The
provider step resumes a profile that already holds a key on the selected BoBe
instead of asking for it again.

## Validation and remaining limits

[Test scope](../CONTRIBUTING.md#test-scope) describes validation. Native Mac,
hardware, external accounts and deployed signed recovery require their actual
environments.

Unsafe Rust is confined to existing OS/FFI boundaries. Live descriptors own
locks; bounded buffers own hostname data. Pairing handles and buffers are
destroyed once through their ABI exports.

Public API versioning, hosted tenancy/isolation, new mobile surfaces, consumer
body enrollment/reset and broader device sensing are unresolved product work.
