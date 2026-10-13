# BoBe product contract

BoBe is one person's continuously running companion. The product invariant is
**one person, one BoBe, one durable agent identity, many possible surfaces and
model providers**.

The Mac app, Rust daemon, pi worker and [ESP32 SDK](esp32-sdk/README.md) share
this repository. The board SDK has its own build and examples. It does not move
companion authority or model execution onto a device.

This document defines the current product boundaries.
[architecture.md](docs/architecture.md) records ownership and
[the iPhone guide](docs/iphone.md) describes that surface's current capabilities.

## The companion

BoBe is one long-lived, sessioned agent. Typed, spoken, proactive, CLI, remote,
and physical-body turns all reach that same identity, memory, skills, tools, and
conversation authority. A surface may sense, schedule, or present, but it does
not become another BoBe.

The daemon is BoBe's brain. It runs standalone on Linux or macOS, or under the
Mac app in managed-local mode, and remains the only authority. Each surface is
a body: the Mac app, a phone, a physical device or the owner CLI. A body
contributes senses, expression and the actions its owner grants. A sense
belongs to the body that has it, so it works the same whether the brain runs
on that machine or elsewhere.

Communication channels have separate persistent conversation histories and
model contexts. The Mac, a phone, the owner CLI and each enrolled body can
continue their own thread across reconnects. They share one workspace, memory,
goals, skills, tools and authority; continuity across bodies comes from that
shared state. Relevant earlier conversations are recalled explicitly;
transcripts are not automatically concatenated across channels.

BoBe has one voice. The brain synthesizes her speech, so every body that speaks
sounds the same. The Mac and physical bodies speak; the iPhone app shows her
replies as text. Speech recognition
runs on the body when the body can do it well, so only words leave the device
and turn-taking stays immediate; the brain recognizes speech for bodies that
cannot. Raw audio is not stored.

BoBe is made for people who are not technical. Every choice has a default that
works without explanation. Setup asks only what BoBe cannot decide for the
owner, and everything can be adapted later in Settings.

BoBe uses the pi harness for the model/tool loop, context management, skills,
parallel tool execution, delegation, streaming, cancellation, session lifecycle
and authentication. Use upstream pi capabilities and licensed, maintained
integrations. A bare model API wrapper is not the full harness.

Provider profiles select models and credentials independently of companion
identity. Profiles support BoBe subscription access through Rail Relay,
user-configured OpenAI, Anthropic, DeepSeek and OpenRouter access, and local
models such as Ollama or other compatible endpoints. Rail Relay access uses an
owner-pasted token stored by the selected authority. Provider subscription
access must use supported authentication and terms; it is not interchangeable
with API billing. A local endpoint needs no cloud login or provider key when
the endpoint requires none.

Changing provider must preserve the companion's capabilities and continuity.
Provider-specific authentication, catalogs, wire adapters, and local-runtime
installation are integration concerns, not separate companion implementations.

## Agency and tools

BoBe is autonomous by design. The main agent and its delegated agents receive
the pi integration's tool, skill, filesystem, shell and MCP capabilities.
Enabled tools run without an interactive approval queue, so background and
proactive work do not stall waiting for a click. Provider and operating-system
access rules still apply.

BoBe-owned domain operations remain explicit where the daemon is the durable
authority—for example memory and goal mutations. The agent may create and
maintain goals proactively when evidence supports a durable, personally
meaningful direction; this does not require a separate approval lifecycle.
General reasoning and workflows belong in skills and the agent harness, not in
Rust decision trees or duplicate custom tool systems.

The stable agent workspace is `workspace/` under the authority's data root. It
is the agent runtime's working directory and is included in backup and restore.
Harness transcript persistence and context processing belong to that authority.
Main and delegated filesystem/shell work uses the owner-visible workspace
rather than hidden per-provider working copies. Host shell
execution has the permissions of the daemon's operating-system account;
local-first autonomy, not hostile multi-tenant isolation, is the current
product boundary.

The owner selects built-in capabilities with `agent.enabled_tools`; the same
selection applies to the companion, scoped subagents, and durable background
jobs after a daemon restart. Memory, goals and MCP have their own capability
controls. Turning off a named tool does not create a filesystem or
network sandbox.

Scoped subagents return within their parent turn. Durable background jobs can
continue after the reply, under a bounded Rust-owned queue. Each job has its
own task transcript, not another companion identity. Results return through
the originating conversation and the shared admission path. Cancellation settles
execution; interrupted external effects are not blindly replayed. Active goals
may automatically reconsider interrupted work on a later review, using the
retained task transcript and checking effects first. Ordinary job controls still
allow explicit owner cancellation and resume.

Channels admit one conversational run at a time, not one run across the whole
companion. Incoming messages are durably queued in order. Human input takes
precedence over deferrable proactive replies, with cancellation settled before
the next run starts. Explicit guidance may steer an active pi run or follow it;
an ordinary new message remains its own correlated turn.

The daemon enforces aggregate inference, tool and process capacity. Task
timeouts and turn limits can be chosen at runtime beneath owner ceilings.
Idle channel execution objects may retire without resetting their durable
session pointer or history. Native shell, search and MCP children are owned
outside Node, so a worker crash does not orphan their active work.

The agent may create once-only or repeated future work through `bobe_schedule`.
The daemon owns due times and atomic job enqueue. Restart preserves the hourly
goal clock and scheduled intent; missed intervals do not replay a burst of
uncertain external actions.

Rare important questions use `bobe_owner_input`: an exact question, action and
continuation are saved, and execution settles while waiting. Native controls,
the owner CLI and the last actually presented conversation question can receive
the answer. A queued message cannot answer a question presented after it arrived.
Only one continuation is released. Routine retries, tool permissions and ordinary
failures are not owner decisions.

A committed response is not proof of presentation. Saved updates can be shown
again without another model call; native visible chat, CLI output and completed
speech playback provide channel-scoped receipts.

The brain's own tools execute on its host. A body adds actions within limits
its owner can change for that body. A Mac body can read its screen on request.
Safe defaults apply without a
setup decision or per-action approval; the owner can narrow or widen them per
body later. The body shows when BoBe uses it and may refuse, and its operating
system's permissions still apply. Presenting a conversation never gives the
brain a body's files, shell or screen.

## Senses, time, goals, and proactivity

A language-model conversation does not provide a reliable clock, sensing loop,
presence model, cooldown policy, or goal scheduler. The daemon therefore gives
BoBe observation, work and communication paths outside any one channel session:

- screen and physical sensors collect bounded observations;
- hourly goal review can start quiet work without an open app or reachout approval;
- every goal ends: it is completed at its ending point, paused when it cannot be achieved, and its reviews rest after three in a row that change nothing;
- screen observation defaults to every two minutes; the Mac body reads its own screen's text, discards the image and sends only that text, which supplies context, not a goal;
- admission and cooldown policy govern communication, not permission to make quiet progress;
- bodies report presence; proactive offers go to the body the owner is at, and other bodies stay quiet;
- bounded evaluators may decide whether an observation or scheduled event is
  meaningful and timely enough to activate the companion;
- an accepted activation becomes contextual input to the same main BoBe agent,
  which decides what to say and what tools or subagents to use;
- the normal conversation/event/presentation path delivers the result.

These systems are BoBe's senses, clock, activation policy, and initiative. They
are not rival companion agents. They exist so BoBe can notice, work, and speak
without waiting for the owner to send a message.

Background work should not unnecessarily stop an active user turn. Auxiliary
observation and evaluation may run independently; a proactive conversational
turn enters the canonical turn-admission path only when it is ready to activate
BoBe. User interaction takes precedence over deferrable background work.

An active goal supplies guidance, not a fixed Rust task plan. Pi decides what
useful authorized work to do, what needs to wait, and which missing preference
or consent genuinely needs the person. Routine technical failures should not
become approval requests. Goal outcomes persist before a separate small-model
decision chooses whether to offer an update now, retain it for later, or stay
silent. A connected surface is a delivery opportunity, not proof of attention.
Idle bodies are not made to speak without a valid speech interaction.

## Bounded auxiliary inference

A small number of operations need a quick model result but are not open-ended
companion turns. Examples include summarizing on-screen text, conversation
summarization, proactive activation evaluation, and memory consolidation. They have known
inputs, bounded outputs, finite lifetimes, and no independent identity.

These operations use the pi integration and a configured workload model.
They may use harness capabilities when useful, but their sessions are auxiliary:
they do not become a second durable BoBe, replace the main conversation session,
or introduce provider-specific hand-written loops. Their result may be supplied
to the main agent or committed by deterministic ownership code when that is the
operation's declared purpose.

The independently configurable chat, goal-work, small-decision/summary and
consolidation routes choose models for workloads, not companion identities.
Provider profiles remember those choices. Model IDs come from actual provider
access; a name such as Luna is not assumed to exist on every provider.

## Deterministic authority

Rust owns the boundaries a model must not improvise:

- runtime identity and single-writer persistence;
- memory, goals, configuration, secrets, and backup/restore;
- observation policy, clocks, trigger cadence, cooldowns, routing, and turn admission;
- endpoint trust, networking, event delivery, voice, and BodyLink protocols;
- tool registration and durable domain mutations;
- cancellation, lifecycle drain, and process supervision.

Rust should not reproduce the harness's agent loop, parallel tool dispatcher,
subagent system, context manager, or general workflow reasoning. When behavior
can be expressed through context, a skill, and the harness's normal tools, empower
BoBe to perform it instead of adding another deterministic orchestration layer.

## Rejected product shapes

- A passive chatbot that acts only after a user message.
- One agent runtime per model provider.
- A second Swift-, surface-, body-, trigger-, or scheduler-owned companion.
- A sense or action that works only when the brain shares the body's machine.
- A body that speaks for BoBe with a voice of its own.
- Long-lived auxiliary sessions that acquire their own companion identity,
  memory, or behavioral prompt.
- Provider selection that changes whether BoBe has skills, tools, parallelism,
  subagents, persistence, or autonomy.
- Routine interactive approval prompts that make autonomous work wait for a
  person.
- Reimplementing harness capabilities as provider-specific loops or duplicate
  general-purpose tools.
