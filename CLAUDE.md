# BoBe

BoBe is a local-first proactive companion. A Rust daemon owns identity, memory,
goals, deterministic policy, tools, persistence, and lifecycle authority. The
daemon runs standalone on Linux or macOS. Native Mac and iPhone apps are surfaces
of that companion, not additional runtimes. The Mac app owns its presentation,
screen reading, microphone capture, speech recognition, and playback.
It supervises the daemon only in managed-local mode; attached-local,
remote, launchd, and systemd runtimes own their own lifecycle.

The product invariant is **one person, one BoBe, one durable agent identity,
many possible surfaces and providers**. A surface may sense or present, and a
provider may supply inference, but neither may fork the companion's authority.

## Working in this repository

Source and tests are authoritative. Start from the relevant implementation and
use documentation to understand intent, not as a substitute for reading code.
Derive validation from the changed behavior; no single command proves every surface.

Read `PRODUCT.md` for current product boundaries and the relevant guide in `docs/`
for supported behavior. BoBe uses the pi harness. Preserve one reasoning runtime
and remove obsolete compatibility paths.
Do not add GitHub Actions, orchestration workflows, or
dependencies without an explicitly agreed product need. Source code contains no
prose comments; keep ownership and safety constraints in current docs, and
preserve functional compiler/tool directives.

Documents describe the current system: what it does, how to use, build and operate it, and each decision that shapes it, with its reason. Change history belongs in commit messages and pull requests. Keep a sentence only when a reader needs it to act correctly.

Surface an unknown before implementation when its answer could change user
behavior, data ownership, an API/wire contract, security, or architecture.
Otherwise use local evidence and proceed.

BoBe's audience is not technical. Every new choice ships with a default that
works without explanation; setup asks only what cannot be defaulted, and
adjustments live in Settings. Do not turn a product decision into an owner
setting.

Start with a blind-spot pass when work reaches the unvalidated Ubuntu path,
firmware or hardware qualification, or hosted isolation.

## Product validation

Full-product E2E checks are disposable agent work, not repository automation.
Frameworks such as `tester-army/e2e` may be used from temporary directories for
one-off inspection of the actual Linux dev build, CLI, APIs and agent loop.
Do not commit generated E2E scripts or add them to CI, builds, package tasks or
the development/test lifecycle. Remove temporary checks after use. Focused
source regression tests remain separate from this manual product evidence.

Validate through the actual `bobe` CLI and daemon authority. Unit tests and
cross-language wire checks catch regressions but are never accepted as evidence
that provider authentication, model execution, streaming, tools, persistence,
or recovery work in the running product. Do not add synthetic headless smoke
scripts as substitutes for operating BoBe itself.

For headless work, validate end to end with a persistent owner authority:

1. start the real daemon and inspect readiness with `bobe status`/`bobe doctor`;
2. authenticate the selected provider through its real CLI flow;
3. select real models and complete initialization;
4. converse through `bobe chat`;
5. invoke the changed capability through conversation or its owner CLI command;
6. inspect the resulting memory, goals, skills, workspace, database, and logs;
7. restart the daemon and verify durable behavior again when persistence or
   lifecycle changed.

Provider API keys belong to the authority's headless secret store. Pi
subscription credentials belong under `agent/auth/`, not a global CLI home.
Use `bobe provider auth login --profile <id>` for provider login and verify the
same account after context resets. Keep any reusable validation credentials
under `$HOME/.local/share/bobe-validation-auth/pi/` before replacing a
validation data root. Both roots must remain owned by the account running validation, with
directories mode `0700` and files mode `0600`. Restore credentials only into
the intended validation authority. Never copy credentials into the repository,
logs, fixtures, command output, or test artifacts.

## Repository gotchas

- Distinguish implemented behavior from proposed platform or hardware direction.
- The daemon is loopback-only by default; remote operation is explicit,
  authenticated, and TLS-protected.
- Typed, spoken, proactive, and physical-body turns share one authoritative
  runtime, memory, goals, workspace, and admission policy. Communication channels
  have separate persistent histories and pi contexts; use explicit recall for
  relevant history from another channel, not automatic transcript copying.
- Provider profiles choose model access, not a companion identity. Profiles
  cover BoBe subscription access through Rail Relay and user-configured
  OpenAI, Anthropic, DeepSeek, OpenRouter and local models. Rail Relay accepts an
  owner-pasted token and has no company sign-in flow. Verify
  provider authentication support and subscription terms rather than assuming an API key and a
  consumer subscription are interchangeable. Anthropic is API-key-only in
  BoBe: its published policy does not permit third-party Claude subscription
  login, even though upstream pi contains an OAuth implementation.
- Summarization, proactive activation evaluation, and memory
  consolidation may use finite harness sessions because they are bounded operations,
  not additional companion identities.
- Preserve filesystem, shell, Git, MCP, skills, parallel tools and delegation in
  the pi integration. Check which capabilities come from upstream pi and which
  need maintained extensions. Routine permissions are automatically approved;
  do not add workspace allowlists or an owner skill-approval queue.
- SSE uses bounded per-client subscriptions, incarnation-scoped cursors and
  snapshot recovery. Conversation text/tool events and snapshots are scoped
  to the authenticated communication channel; runtime health remains shared.
- Rust and Swift wire contracts are manually mirrored and checked for drift.
- Companion behavior belongs in the existing worker skill or a bounded
  turn-specific hint; do not create a parallel prompt layer.
- The Mac supports managed-local, attached-local, and remote endpoint profiles;
  only managed-local owns daemon spawn, stop, and restart. A managed-local
  daemon exits when the app process that started it is gone.
- Each channel has a stable pi pointer across restart, tool/skill/MCP changes,
  compaction and provider-route changes. All channels use the same workspace,
  memory, goals and credentials.
- Incoming messages are durably queued per channel. Human-priority cancellation
  must settle the previous run before releasing admission. Explicit steering is
  not an ordinary new conversation message.
- Schedules and hourly goal clocks are durable daemon intents; job enqueue and
  next-due advancement are atomic. Important owner questions save their exact
  action and release execution while waiting. Never infer an answer from a
  message received before that question was actually presented.
- Result commitment is distinct from presentation. Re-show stored output without
  rerunning inference. Shell/search/MCP processes belong to Rust, not Node.
- Goal work is separate from reachout. Active goals are reviewed hourly by
  default; screen observation defaults to two minutes. Quiet work does not wait
  for the owner to open the app. Reviews of a goal rest after three in a row
  that leave its document unchanged and resume when it changes; the starter
  example goal is created paused. Small-model update decisions can offer, defer,
  or suppress an update; ordinary failures are not owner approval requests.
- One canonical `DataPaths` plus process-lifetime authority/database locks protect
  each runtime.
- Headless provider credentials use `headless-secrets.json` on Unix, including
  macOS launchd or SSH operation; pi login state uses `agent/auth/`. Do not
  assume every daemon secret is in Keychain.
- CLI backup/restore and signed update/rollback mechanics are implemented, but
  deployed Ubuntu x86_64 recovery and release qualification remain evidence gates.
- UI tests use `BOBE_DEV=1` and `-bobe.onboarding_completed NO` to enter setup.

## References

- `PRODUCT.md`: authoritative implemented product shape and rejected split-brain designs.
- `docs/architecture.md`: implemented architecture and present constraints.
- `docs/iphone.md`: the iPhone surface, pairing and current limitations.
- `docs/physical-bobe.md`: physical product behavior, acoustic gates and embodiment boundaries.
- `esp32-sdk/README.md`: board SDK setup, supported targets, examples and device interfaces.
- `docs/device-pairing-crypto.md`: pairing protocol and unsafe C ABI constraints.
- `docs/cli.md`: standalone operation, signed updates and recovery commands.
- `docs/releasing.md`: release build, signing, notarization and publication.
- `BoBeAgent/assets/chat.md`: the single shipped companion behavior source, embedded by Rust and loaded by pi.
- `BoBeService/migrations/schema.sql`: persisted SQLite contract.
- `scripts/check-cross-language-constants.sh`: Rust/Swift wire drift checks.
- `scripts/check-bodylink-contract.sh`: Rust/Swift/firmware BodyLink contract.
- `BoBeMacUI/Tests/BoBeTests/VoiceProtocolTests.swift`: executable voice wire reference.
