# BoBe

BoBe is a local-first proactive companion. A Rust daemon owns identity, memory,
goals, deterministic policy, tools, persistence and lifecycle. It runs standalone
on Linux or macOS. The Mac and iPhone apps are surfaces of that companion, not
additional runtimes. The Mac app owns presentation, screen reading, microphone
capture, speech recognition and playback. It supervises the daemon only in
managed-local mode; attached-local, remote, launchd and systemd runtimes own
their lifecycle.

The invariant is **one person, one BoBe, one durable agent identity, many
surfaces and providers**. A surface senses or presents and a provider supplies
inference; neither may fork the companion's authority.

## Working here

- Source and tests are authoritative; documentation explains intent. Derive
  validation from the changed behavior.
- Read `PRODUCT.md` for product boundaries and the relevant guide in `docs/`.
- BoBe uses the pi harness. Keep one reasoning runtime and remove obsolete
  compatibility paths.
- Add dependencies, workflows or orchestration only for an agreed product need.
  The only CI is `.github/workflows/ci.yml`; it runs on demand, not on every
  pull request.
- Source code contains no prose comments. Keep ownership and safety constraints
  in current documentation and preserve functional compiler and tool directives.
- Documents describe the current system and the reason for each decision.
  Change history belongs in commits and pull requests. Keep a sentence only when
  a reader needs it to act correctly.
- Raise an unknown before implementation when its answer could change user
  behavior, data ownership, an API or wire contract, security or architecture.
  Otherwise use local evidence and proceed.
- The audience is not technical. Every new choice ships with a default that works
  without explanation; setup asks only what cannot be defaulted, and adjustments
  live in Settings. Do not turn a product decision into an owner setting.
- Start with a blind-spot pass when work reaches the unvalidated Ubuntu path,
  firmware or hardware qualification, or hosted isolation.
- Private plans, operator records and secrets belong in the private operations
  repository, not here.

## Product validation

Unit tests and Rust/Swift wire checks catch regressions. They are not evidence
that provider authentication, model execution, streaming, tools, persistence or
recovery work. Validate through the real `bobe` CLI and daemon:

1. start the daemon and check `bobe status` and `bobe doctor`;
2. authenticate the provider through its real flow
   (`bobe provider auth login --profile <id>`);
3. select models and complete initialization;
4. converse through `bobe chat`;
5. invoke the changed capability through conversation or its CLI command;
6. inspect the resulting memory, goals, skills, workspace, database and logs;
7. restart and check again when persistence or lifecycle changed.

End-to-end checks are disposable. A framework such as `tester-army/e2e` may run
from a temporary directory against the real build; never commit it or add it to
CI, builds or package tasks, and remove it afterwards. Do not substitute
synthetic headless smoke scripts for operating BoBe.

Provider API keys belong to the authority's headless secret store; pi
subscription credentials belong under `agent/auth/`. Credential directories are
mode `0700` and files `0600`. Never copy credentials into the repository, logs,
fixtures, command output or test artifacts.

## Repository gotchas

- Distinguish implemented behavior from proposed platform or hardware direction.
- The daemon is loopback-only by default; remote operation is explicit,
  authenticated and TLS-protected.
- Typed, spoken, proactive and physical-body turns share one runtime, memory,
  goals, workspace and admission policy. Each communication channel has its own
  persistent history and pi context; recall another channel's history
  explicitly instead of copying transcripts.
- Provider profiles choose model access, not a companion identity: Rail Relay
  with an owner-pasted token, OpenAI, Anthropic, DeepSeek, OpenRouter and local
  models. Verify each provider's authentication support and subscription terms.
  Anthropic is API-key-only in BoBe because its policy does not permit
  third-party Claude subscription login, although upstream pi implements OAuth.
- Summarization, proactive activation evaluation and memory consolidation may
  use finite harness sessions; they are bounded operations, not identities.
- Preserve filesystem, shell, Git, MCP, skills, parallel tools and delegation in
  the pi integration, and know which come from upstream pi and which are
  maintained extensions. Routine permissions are approved automatically; do not
  add workspace allowlists or an owner skill-approval queue.
- SSE uses bounded per-client subscriptions, incarnation-scoped cursors and
  snapshot recovery. Conversation events and snapshots are scoped to the
  authenticated channel; runtime health is shared.
- Rust and Swift wire contracts are mirrored by hand and checked for drift.
- Companion behavior belongs in the worker skill or a bounded turn-specific
  hint, never a parallel prompt layer.
- The Mac supports managed-local, attached-local and remote endpoints. Only
  managed-local spawns, stops and restarts the daemon, and that daemon exits
  when the app that started it is gone.
- Each channel keeps a stable pi pointer across restart, tool, skill and MCP
  changes, compaction and provider changes. All channels share the workspace,
  memory, goals and credentials.
- Incoming messages are queued durably per channel. Human-priority cancellation
  settles the previous run before releasing admission. Explicit steering is not
  an ordinary conversation message.
- Schedules and hourly goal clocks are durable daemon intents; job enqueue and
  next-due advancement are atomic. An important owner question saves its exact
  action and releases execution while waiting. Never infer an answer from a
  message received before the question was presented.
- Committing a result is distinct from presenting it. Re-show stored output
  without rerunning inference. Shell, search and MCP processes belong to Rust,
  not Node.
- Goal work is separate from reachout. Active goals are reviewed hourly and the
  screen every two minutes by default, without waiting for the app to open. A
  goal's reviews rest after three in a row leave its document unchanged and
  resume when it changes; the starter goal is created paused. Small-model update
  decisions offer, defer or suppress an update; ordinary failures are not owner
  approval requests.
- One canonical `DataPaths` and process-lifetime authority and database locks
  protect each runtime.
- On Unix, including launchd and SSH operation, headless provider credentials
  are in `headless-secrets.json` and pi login state is in `agent/auth/`. Not
  every daemon secret is in the Keychain.
- Backup, restore and signed update and rollback are implemented; deployed
  Ubuntu x86_64 recovery and release qualification remain evidence gates.
- UI tests use `BOBE_DEV=1` and `-bobe.onboarding_completed NO` to enter setup.

## References

- `PRODUCT.md`: implemented product shape and rejected split-brain designs.
- `docs/architecture.md`: architecture and present constraints.
- `docs/speech.md`: speech engines, models and their licenses.
- `docs/iphone.md`, `docs/physical-bobe.md`: the other surfaces.
- `esp32-sdk/README.md`: board SDK, targets, examples and device interfaces.
- `docs/device-pairing-crypto.md`: pairing protocol and unsafe C ABI constraints.
- `docs/cli.md`: standalone operation, signed updates and recovery.
- `docs/releasing.md`: release build, signing, notarization and publication.
- `BoBeAgent/assets/chat.md`: the single companion behavior source, embedded by
  Rust and loaded by pi.
- `BoBeService/migrations/schema.sql`: persisted SQLite contract.
- `scripts/check-cross-language-constants.sh`,
  `scripts/check-bodylink-contract.sh`: Rust, Swift and firmware drift checks.
- `BoBeMacUI/Tests/BoBeTests/VoiceProtocolTests.swift`: executable voice wire
  reference.
- `TRADEMARKS.md`: use of the BoBe name, logo and character.
