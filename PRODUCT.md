# BoBe product contract

BoBe is one person's continuously running companion: **one person, one BoBe, one
durable agent identity, many possible surfaces and model providers**. The Mac
app, Rust daemon, pi worker and [ESP32 SDK](esp32-sdk/README.md) share this
repository; the SDK builds devices and never moves companion authority or model
execution onto one. [Architecture](docs/architecture.md) records ownership and
[iPhone](docs/iphone.md) that surface's capabilities.

## The companion

The daemon is BoBe's brain and the only authority. It runs standalone on Linux
or macOS, or under the Mac app in managed-local mode. Typed, spoken, proactive,
CLI, remote and physical-body turns reach the same identity, memory, skills,
tools and conversation authority. The Mac app, a phone, a physical device and
the CLI are bodies: they contribute senses, expression and actions their owner
grants, and never become another BoBe. A sense belongs to the body that has it
and works the same wherever the brain runs.

Each communication channel keeps its own persistent history and model context,
and continues across reconnects. All channels share the workspace, memory,
goals, skills, tools and authority. Earlier conversations are recalled
explicitly; transcripts are never concatenated across channels.

BoBe has one voice, synthesized by the brain, so every body that speaks sounds
the same; the iPhone shows replies as text. A body recognizes speech itself when
it can do it well, so only words leave it; the brain recognizes speech for the
rest. Raw audio is not stored.

BoBe is for people who are not technical. Every choice has a working default;
setup asks only what BoBe cannot decide, and everything can be changed later in
Settings.

The pi harness provides the model and tool loop, context management, skills,
parallel tools, delegation, streaming, cancellation, session lifecycle and
authentication. Use upstream pi and licensed, maintained integrations; a bare
model API wrapper is not the harness.

Provider profiles choose models and credentials, not identity: Rail Relay with
an owner-pasted token, OpenAI, Anthropic, DeepSeek, OpenRouter, and local
endpoints such as Ollama, which need no cloud login. Subscription access must use
supported authentication and terms and is not interchangeable with API billing.
Changing provider preserves capabilities and continuity; provider
authentication, catalogs, wire adapters and local-runtime installation are
integration concerns.

## Agency and tools

BoBe is autonomous. The main agent and its delegates get pi's tools, skills,
filesystem, shell and MCP. Enabled tools run without an approval queue, so
background work never waits for a click; provider and operating-system rules
still apply.

The daemon stays the explicit authority for its domain operations, such as
memory and goal changes. The agent may create and maintain goals when evidence
supports a durable, personally meaningful direction, without a separate
approval lifecycle. General reasoning belongs in skills and the harness, not in
Rust decision trees or duplicate tool systems.

The agent works in `workspace/` under the authority's data root; it is included
in backup and restore, and main and delegated work use it rather than hidden
per-provider copies. The authority owns harness transcripts and context
processing. Shell commands run with the daemon account's permissions: the
boundary is local-first autonomy, not multi-tenant isolation.

`agent.enabled_tools` selects built-in capabilities for the companion,
subagents and background jobs after a restart; memory, goals and MCP have their
own controls. Disabling a tool does not create a filesystem or network sandbox.

Subagents return within their parent turn. Durable background jobs continue
after the reply in a bounded Rust-owned queue, each with its own task transcript
and no separate identity, and report back through the originating conversation
and the shared admission path. Cancellation settles execution; interrupted
external effects are not replayed blindly. A later goal review may reconsider
interrupted work from its transcript after checking effects. Owners can still
cancel and resume jobs.

Each channel admits one conversational run at a time. Incoming messages queue
durably in order; human input preempts deferrable proactive replies, with
cancellation settled first. Explicit guidance may steer or follow an active run;
an ordinary message is its own turn.

The daemon enforces aggregate inference, tool and process capacity, with
runtime timeouts and turn limits under owner ceilings. Idle channel execution
objects may retire without losing their session pointer or history. Shell,
search and MCP processes are owned outside Node, so a worker crash orphans
nothing.

`bobe_schedule` creates one-off or repeated future work. The daemon owns due
times and atomic enqueue; restarts preserve the hourly goal clock and scheduled
intent, and missed intervals are not replayed as a burst.

`bobe_owner_input` asks a rare important question: the exact question, action
and continuation are saved and execution settles while waiting. Native
controls, the CLI or the last question actually presented in conversation can
answer it, never a message queued before it was presented, and only one
continuation is released. Retries, tool permissions and ordinary failures are
not owner decisions.

Committing a response is not presenting it. Saved updates can be shown again
without another model call; visible chat, CLI output and completed playback give
channel-scoped receipts.

The brain's tools run on its host. A body adds actions within limits its owner
can change per body, with safe defaults and no per-action approval; a Mac body
can read its screen on request. The body shows when BoBe uses it and may refuse,
and its operating system's permissions apply. Presenting a conversation never
gives the brain a body's files, shell or screen.

## Senses, time, goals and proactivity

A model conversation has no reliable clock, sensing loop, presence model,
cooldown or goal scheduler, so the daemon provides them outside any channel:

- screen and physical sensors collect bounded observations; the Mac reads its
  screen's text every two minutes by default, discards the image and sends only
  the text, as context rather than a goal;
- hourly goal review starts quiet work without an open app or approval;
- every goal ends: completed at its end point, paused when unachievable, and its
  reviews rest after three in a row change nothing;
- admission and cooldowns govern speaking, not quiet progress;
- bodies report presence, and proactive offers go only to the body the owner is
  at;
- bounded evaluators decide whether an observation or event should activate the
  companion; an accepted activation becomes input to the same main agent, which
  decides what to say and do, and the result is delivered on the normal path.

These are BoBe's senses, clock and initiative, not rival agents. Background
work should not stop an active user turn; a proactive turn enters admission only
when ready, and user interaction comes first.

An active goal guides pi; it is not a fixed Rust plan. Pi decides what
authorized work to do, what waits, and which preference or consent truly needs
the person; technical failures are not approval requests. Goal outcomes persist
before a small-model decision offers an update now, keeps it, or stays silent. A
connected surface is a delivery opportunity, not proof of attention, and idle
bodies do not speak without a valid speech interaction.

## Bounded auxiliary inference

Screen-text summaries, conversation summaries, activation evaluation and memory
consolidation need quick model results with known inputs, bounded outputs and
finite lifetimes. They use pi and a configured workload model, never become a
second BoBe, replace the main session or add provider-specific loops. Their
results go to the main agent or are committed by deterministic code when that
is their declared purpose.

The chat, goal-work, small-decision and consolidation routes choose models for
workloads, not identities, and provider profiles remember them. Model IDs come
from actual provider access; no name is assumed to exist everywhere.

## Deterministic authority

Rust owns the boundaries a model must not improvise:

- runtime identity and single-writer persistence;
- memory, goals, configuration, secrets, backup and restore;
- observation policy, clocks, triggers, cooldowns, routing and admission;
- endpoint trust, networking, event delivery, voice and BodyLink;
- tool registration and durable domain changes;
- cancellation, lifecycle drain and process supervision.

Rust does not reproduce the harness's agent loop, parallel tools, subagents,
context management or general reasoning. What context, a skill and normal tools
can express belongs to BoBe, not another orchestration layer.

## Rejected product shapes

- A passive chatbot that acts only after a user message.
- One agent runtime per model provider.
- A second companion owned by Swift, a surface, a body, a trigger or a scheduler.
- A sense or action that works only when the brain shares the body's machine.
- A body that speaks with a voice of its own.
- Long-lived auxiliary sessions with their own identity, memory or prompt.
- Provider choice that changes skills, tools, parallelism, subagents,
  persistence or autonomy.
- Routine approval prompts that make autonomous work wait.
- Harness capabilities rebuilt as provider-specific loops or duplicate tools.
