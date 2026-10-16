# BoBe

BoBe is a local-first proactive companion: **one person, one BoBe, one durable
agent identity, many surfaces and providers**. The Rust daemon is the only
authority (identity, memory, goals, policy, tools, persistence, lifecycle) and
runs standalone on Linux or macOS. The Mac and iPhone apps and ESP32 bodies are
surfaces, and a provider only supplies inference; neither may fork authority.

Read [PRODUCT.md](PRODUCT.md) and [docs/architecture.md](docs/architecture.md)
before changing behavior. Commands are in [CONTRIBUTING.md](CONTRIBUTING.md).

## Rules

- Source and tests are authoritative; distinguish implemented behavior from
  proposed direction.
- One reasoning runtime (pi). Remove obsolete compatibility paths.
- Add dependencies, workflows or orchestration only for an agreed product
  need. The only CI is `.github/workflows/ci.yml`, run on demand.
- Source code has no prose comments; keep functional directives. Docs describe
  the current system only; history belongs in commits.
- Raise an unknown before implementing when it could change user behavior,
  data ownership, an API or wire contract, security or architecture;
  otherwise decide from local evidence.
- The audience is not technical: every choice ships with a working default,
  setup asks only what cannot be defaulted, and adjustments live in Settings.
- Start with a blind-spot pass for the Ubuntu path, firmware or hardware
  qualification, or hosted isolation.
- Private plans, operator records and secrets belong in the private operations
  repository.

## Invariants

- A change to `BoBeService/migrations/schema.sql` raises the schema version and
  needs an `upgrade-<from>-to-<to>.sql` step plus a test that upgrades the
  previous fixture. User data must survive every update.
- Rust, Swift and firmware wire contracts are mirrored by hand: change them
  together and run `scripts/check-cross-language-constants.sh` and
  `scripts/check-bodylink-contract.sh`.
- Companion behavior lives only in `BoBeAgent/assets/chat.md`. Rust sends
  events as markers plus data, never instruction prose; owners customize BoBe
  through souls and user profiles.
- Every channel has its own history and pi session; all share workspace,
  memory, goals and credentials. Recall other channels explicitly.
- Rust owns every policy value, durable job, schedule, owner question and
  shell, search or MCP process. Never infer an owner's answer from a message
  sent before the question was shown.
- Keep pi's filesystem, shell, Git, MCP, skills, parallel tools and
  delegation. Routine permissions are approved automatically; add no
  allowlists or approval queues.
- Provider profiles choose model access, not identity; Anthropic is
  API-key-only.
- The daemon is loopback-only by default; remote access is explicit and TLS.
  Only managed-local Macs start or stop the daemon.

## Validation

Unit tests and wire checks do not prove that providers, streaming, tools,
persistence or recovery work. Operate the real build: start the daemon and
check `bobe status` and `bobe doctor`; sign in through the real flow; select
models and finish initialization; talk through `bobe chat`; exercise the
changed capability; inspect memory, goals, skills, workspace, database and
logs; restart and check again when persistence or lifecycle changed.

End-to-end tools run from a temporary directory and are never committed. UI
tests enter setup with `BOBE_DEV=1` and `-bobe.onboarding_completed NO`. API
keys live in `headless-secrets.json` and pi sign-ins under `agent/auth/`
(directories `0700`, files `0600`); never copy credentials into the repository,
logs, fixtures, output or test artifacts.
