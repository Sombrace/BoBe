# Contributing to BoBe

BoBe is a Rust daemon, a pi agent worker and a native Swift macOS app. Start with
[Getting started](docs/getting-started.md), [Using BoBe](docs/using-bobe.md) and
the [documentation index](docs/README.md). [AGENTS.md](AGENTS.md) has the
repository rules and gotchas. Bugs and ideas go to
[Issues](https://github.com/Sombrace/BoBe/issues); security reports follow
[SECURITY.md](SECURITY.md).

## Setup

You need macOS 26+, Xcode 26+ with Swift 6, and [rustup](https://rustup.rs);
`rust-toolchain.toml` selects Rust 1.98.1. Runtime testing needs a provider API
key, a Rail Relay token, a provider subscription or Ollama.

```bash
git clone https://github.com/Sombrace/BoBe.git
cd BoBe
scripts/tasks/bootstrap-tools.sh
```

Developer tools are pinned under the ignored `.tools/` directory; install nothing
globally. `scripts/tasks/build-agent.sh` downloads Node 22.22.0, verifies its
pinned SHA-256 and builds `BoBeAgent` from its lockfile with lifecycle scripts
disabled. It stages `build/agent-runtime/` with production packages, pi assets
and checksum-pinned ripgrep and fd. Pins are in `scripts/lib/agent-runtime.sh`.

The daemon, pi worker and CLI also build on Linux with
`scripts/tasks/build-backend.sh debug` or `release`. Mac-only Swift, Xcode and
audio checks cannot be signed off from Linux.

To run a daemon by hand, set `BOBE_NODE_PATH` to the staged Node and
`BOBE_AGENT_ENTRY` to `build/agent-runtime/agent/dist/main.js`; the task scripts
do this for you. Provider credentials belong to the selected daemon authority:
signing in to another CLI does not sign BoBe in. The app may restart its
managed-local daemon to load saved provider access; attached-local and remote
daemons are restarted only by their operator.

## Commands

| Command | Purpose |
|---|---|
| `scripts/tasks/check.sh` | The complete Rust, Swift, contract, dependency and packaging gate (macOS) |
| `scripts/tasks/run.sh` | Build and launch the debug app; set `BOBE_DEVELOPMENT_TEAM` to sign it |
| `scripts/tasks/backend.sh` | Run the daemon for Xcode development |
| `scripts/tasks/build-agent.sh` | Build and validate the pinned Node and pi distribution |
| `scripts/tasks/notices.sh` | Write the third-party license notices that releases ship |
| `scripts/check-bodylink-contract.sh` | Check Rust, Swift and SDK wire constants |
| `scripts/tasks/xcode.sh` | Regenerate the Xcode project |
| `scripts/tasks/clean.sh` | Remove build output |

[Releasing](docs/releasing.md) covers the release scripts. Task scripts live in
`scripts/tasks/` and checks directly under `scripts/`. Cargo manifests, the Swift
package, `deny.toml` and the lint configurations are the policy authorities.

The [ESP32 SDK](esp32-sdk/README.md) has its own tools. Building the daemon or
the app needs no ESP-IDF; `BOBE_ESP_SDK_ROOT` points the BodyLink contract check
at another SDK tree.

Without `BOBE_DEVELOPMENT_TEAM`, `scripts/tasks/run.sh` builds an unsigned app
with no Keychain groups that keeps its endpoint credential in memory (see
[Signed or unsigned](docs/getting-started.md#signed-or-unsigned)). Work on the
Keychain path, or on pairing that must survive a relaunch, needs a signed build:
`BOBE_DEVELOPMENT_TEAM=<team-id> scripts/tasks/run.sh`.

## Continuous integration

`.github/workflows/ci.yml` runs the Linux half of `check.sh`: the agent, Rust
formatting, Clippy and tests, dependency policy and the cross-language
contracts. It runs when a maintainer adds the `ci` label to a pull request, on
demand, and when code changes on `main`. Run `scripts/tasks/check.sh` on a Mac
for the Swift and packaging half.

## Engineering expectations

- Follow nearby code rather than generic doctrine.
- Rust is an internal binary crate: prefer crate-scoped visibility, structured
  errors, constructor injection and the handler, service and repository
  boundaries.
- Swift uses `@Observable` and Swift concurrency, and AppKit only where window or
  system behavior requires it.
- Source code has no prose comments. Keep architectural, unsafe or FFI,
  lifecycle and protocol constraints in the documentation; keep functional
  directives such as shebangs, Swift tools versions and lint rules.
- Change Rust, Swift, tests and cross-language constants together for any wire
  change.
- Keep loopback-safe defaults and daemon ownership of credentials and durable
  state.

## Tests

Keep tests that protect observable behavior: authority isolation, persistence
and recovery, admission and cancellation, protocol compatibility, privacy and
audio integrity. Use table-driven tests for repetitive cases. Do not test
defaults, incidental wording, a dependency's internals or assertion-free
construction.

Test fixtures may `unwrap` or `expect`; production code keeps those Clippy
warnings. Passing tests do not replace operating the daemon, the Mac app or the
body. The model-backed STT corpus test is ignored by default:

```bash
BOBE_TEST_STT_MODEL_DIR=<dir> BOBE_TEST_STT_WAV=<wav> \
  cargo test --manifest-path BoBeService/Cargo.toml real_streaming_model_transcribes_corpus -- --ignored
```

Native preference fixtures use `EphemeralUserDefaults` with absolute temporary
suites so tests never write to the real preference domain.

End-to-end frameworks are disposable inspection tools: run them from a temporary
directory, remove them afterwards, and never add them to CI, builds or tasks.

## Pull requests

1. Read the current implementation and tests.
2. Make the smallest coherent change.
3. Run the checks the change calls for, including runtime or UI evidence.
4. Describe user-visible behavior, schema, wire and dependency changes, and what
   remains uncertain.

Dependency changes need explicit review when they add proc macros, build
scripts, FFI or `-sys` crates, registries, git sources, subprocesses, parsers,
archives, filesystem access or network reach.

## Distribution

Release signing, notarization, Sparkle publication and credentials are local
maintainer operations ([Releasing](docs/releasing.md)). Never commit release
secrets, key material, generated output, dependency caches or device recovery
data.

Releases ship BoBe's license, [TRADEMARKS.md](TRADEMARKS.md) and the notices
written by `scripts/tasks/notices.sh`. Third-party brand artwork keeps its own
terms.

Contributions are licensed under the [MIT License](LICENSE) and follow the
[Code of Conduct](CODE_OF_CONDUCT.md). A fork you distribute needs its own name
and logo; see [TRADEMARKS.md](TRADEMARKS.md).
