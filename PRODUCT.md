# BoBe product contract

BoBe is one person's continuously running companion: **one person, one BoBe, one
durable agent identity, many possible surfaces and model providers**.
[Architecture](docs/architecture.md) records which part owns what; the
[ESP32 SDK](esp32-sdk/README.md) builds devices and never moves companion
authority or model execution onto one.

## The companion

The daemon is BoBe's brain and the only authority. It runs standalone on Linux
or macOS, or under the Mac app in managed-local mode. Typed, spoken, proactive,
CLI, remote and physical-body turns reach the same identity, memory, skills,
tools and conversation authority. The Mac app, a phone, a physical device and
the CLI are bodies: they contribute senses, expression and actions their owner
grants, and never become another BoBe. A sense belongs to the body that has it
and works the same wherever the brain runs.

Each communication channel keeps its own persistent history and model context
across reconnects. All channels share the workspace, memory, goals, skills,
tools and authority; earlier conversations are recalled explicitly, never
concatenated across channels.

BoBe has one voice, synthesized by the brain, so every body that speaks sounds
the same; the iPhone shows replies as text. A body recognizes speech itself when
it can do it well, so only words leave it; the brain recognizes speech for the
rest. Raw audio is not stored.

BoBe is for people who are not technical. Every choice has a working default;
setup asks only what BoBe cannot decide, and Settings changes everything later.

The pi harness provides the model and tool loop, context management, skills,
parallel tools, delegation, streaming, cancellation, sessions and
authentication; a bare model API wrapper is not the harness. Provider profiles
choose models and credentials, not identity: Rail Relay with an owner-pasted
token, OpenAI, Anthropic, DeepSeek, OpenRouter, and local endpoints such as
Ollama. Subscription access uses supported authentication and terms, and
changing provider preserves capabilities and continuity.

## Agency and tools

BoBe is autonomous. The main agent and its delegates get pi's tools, skills,
filesystem, shell and MCP. Enabled tools run without an approval queue, so
background work never waits for a click; provider and operating-system rules
still apply.

The daemon stays the authority for its domain operations, such as memory and
goal changes. The agent may create and maintain goals when evidence supports a
durable, personally meaningful direction. General reasoning belongs in skills
and the harness, not in Rust decision trees or duplicate tool systems.

The agent works in `workspace/` under the daemon's data root, which backups
include. Shell commands run with the daemon account's permissions: the boundary
is local-first autonomy, not multi-tenant isolation, and disabling a tool does
not create a sandbox.

Subagents return within their parent turn. Background jobs continue after the
reply in a bounded Rust-owned queue, without a separate identity, and report
back through the originating conversation. Interrupted external effects are not
replayed blindly; owners can cancel and resume jobs.

Each channel admits one conversational run at a time. Messages queue durably in
order; human input preempts deferrable proactive replies, and explicit guidance
may steer an active run. The daemon owns due times for scheduled work; restarts
keep the hourly goal clock and scheduled intent, and missed intervals are not
replayed as a burst.

A rare important question is saved with its exact action and continuation, and
execution settles while waiting. Only an answer given after the question was
presented releases it, once. Retries, tool permissions and ordinary failures
are not owner decisions.

Committing a response is not presenting it; saved updates can be shown again
without another model call.

The brain's tools run on its host. A body adds actions within limits its owner
can change per body, with safe defaults and no per-action approval; a Mac body
can read its screen on request. The body shows when BoBe uses it and may refuse.
Presenting a conversation never gives the brain a body's files, shell or screen.

## Senses, time, goals and proactivity

A model conversation has no clock, sensing loop, presence model, cooldown or
goal scheduler, so the daemon provides them:

- the Mac reads its screen's text every two minutes by default, discards the
  image and sends only the text, as context rather than a goal;
- hourly goal review starts quiet work without an open app or approval;
- every goal ends: completed at its end point, paused when unachievable, and its
  reviews rest after three in a row change nothing;
- admission and cooldowns govern speaking, not quiet progress;
- proactive offers go only to the body the owner is at;
- bounded evaluators decide whether an observation should activate the
  companion; an accepted activation becomes input to the same main agent.

These are BoBe's senses, clock and initiative, not rival agents; user
interaction comes first. An active goal guides pi; it is not a fixed Rust plan.
Pi decides what authorized work to do and what truly needs the person, and
technical failures are not approval requests. Goal outcomes persist before a
small-model decision offers an update now, keeps it, or stays silent. Idle
bodies do not speak without a valid speech interaction.

## Bounded auxiliary inference

Screen-text summaries, conversation summaries, activation evaluation and memory
consolidation use pi with a workload model for quick, bounded results; they
never become a second BoBe or replace the main session. The chat, goal-work,
small-decision and consolidation routes choose models for workloads, not
identities, from actual provider access.

## Deterministic authority

Rust owns the boundaries a model must not improvise:

- runtime identity and single-writer persistence;
- memory, goals, configuration, secrets, backup and restore;
- observation policy, clocks, triggers, cooldowns, routing and admission;
- endpoint trust, networking, event delivery, voice and BodyLink;
- tool registration and durable domain changes;
- cancellation, lifecycle drain and process supervision.

Rust does not reproduce the harness's agent loop, parallel tools, subagents,
context management or general reasoning.

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
