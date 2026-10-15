# BoBe

BoBe is a local-first proactive companion: **one person, one BoBe, one durable
agent identity, many surfaces and providers**. The Rust daemon is the only
authority (identity, memory, goals, policy, tools, persistence, lifecycle) and
runs standalone on Linux or macOS. The Mac and iPhone apps and ESP32 bodies
are surfaces; a provider only supplies inference. Neither may fork authority.

Read [PRODUCT.md](PRODUCT.md) and [docs/architecture.md](docs/architecture.md)
before changing behavior. Commands are in [CONTRIBUTING.md](CONTRIBUTING.md).

## Rules

- Source and tests are authoritative. Distinguish implemented behavior from
  proposed direction.
- One reasoning runtime (pi). Remove obsolete compatibility paths.
- Add dependencies, workflows or orchestration only for an agreed product
  need. The only CI is `.github/workflows/ci.yml`, run on demand.
- Source code has no prose comments; keep functional directives. Docs describe
  the current system only; history belongs in commits.
- Raise an unknown before implementing when it could change user behavior,
  data ownership, an API or wire contract, security or architecture.
  Otherwise decide from local evidence.
- The audience is not technical: every choice ships with a working default,
  setup asks only what cannot be defaulted, adjustments live in Settings.
- Start with a blind-spot pass for the Ubuntu path, firmware or hardware
  qualification, or hosted isolation.
- Private plans, operator records and secrets belong in the private
  operations repository.

## Invariants

- Changing `BoBeService/migrations/schema.sql` raises the schema version and
  needs an `upgrade-<from>-to-<to>.sql` step plus a test that upgrades the
  previous version's fixture. Users' data must survive every update.
- Rust, Swift and firmware wire contracts are mirrored by hand. Change them
  together and run `scripts/check-cross-language-constants.sh` and
  `scripts/check-bodylink-contract.sh`.
- Companion behavior lives only in `BoBeAgent/assets/chat.md`, read by the
  worker from its own package. Rust sends events as markers plus data, never
  instruction prose. Owners customize BoBe through souls and user profiles.
- Every channel keeps its own history and pi session; all share workspace,
  memory, goals and credentials. Recall other channels explicitly.
- Rust owns every policy value, durable job, schedule and owner question, and
  the shell, search and MCP processes. Never infer an owner's answer from a
  message sent before the question was presented.
- Preserve pi's filesystem, shell, Git, MCP, skills, parallel tools and
  delegation. Routine permissions are approved automatically; add no
  allowlists or approval queues.
- Provider profiles choose model access, not identity. Anthropic is
  API-key-only.
- The daemon is loopback-only by default; remote access is explicit and TLS.
  Only managed-local Macs start or stop the daemon.

## Validation

Unit tests and wire checks catch regressions; they do not prove that
providers, streaming, tools, persistence or recovery work. Operate the real
build:

1. start the daemon; check `bobe status` and `bobe doctor`;
2. sign in through the real flow (`bobe provider auth login --profile <id>`);
3. select models and finish initialization;
4. talk through `bobe chat`;
5. exercise the changed capability;
6. inspect memory, goals, skills, workspace, database and logs;
7. restart and check again when persistence or lifecycle changed.

End-to-end tools run from a temporary directory and are never committed. UI
tests enter setup with `BOBE_DEV=1` and `-bobe.onboarding_completed NO`.

API keys live in the daemon's `headless-secrets.json` and pi sign-ins under
`agent/auth/` (directories `0700`, files `0600`). Never copy credentials into
the repository, logs, fixtures, output or test artifacts.
