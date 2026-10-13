# Contributing to BoBe

BoBe is a Rust daemon, a pi agent host, and a native Swift macOS application. Product context,
repository gotchas, and reference ownership are in [CLAUDE.md](CLAUDE.md).

New here? [Getting started](docs/getting-started.md) gets BoBe running,
[Using BoBe](docs/using-bobe.md) shows what she does, and
[the documentation index](docs/README.md) explains how the three programs fit
together. Bugs and ideas go to
[Issues](https://github.com/Sombrace/BoBe/issues); security reports go through
[SECURITY.md](SECURITY.md).

## Setup

Native Mac requirements:

- macOS 26+
- Rust 1.98.1+
- Xcode 26+ with Swift 6
- A supported provider API key, Rail Relay token, provider subscription, or Ollama for runtime testing

```bash
git clone https://github.com/Sombrace/BoBe.git
cd BoBe
scripts/tasks/bootstrap-tools.sh
```

Developer tools are pinned under the checkout's ignored `.tools/` directory.
Do not install project tooling globally.

The Rust daemon, pi worker and headless CLI can also be developed on Linux.
Use `scripts/tasks/build-backend.sh debug` or `release`; operate the resulting
`bobe` executable against a private authority. Mac-only Swift/Xcode/audio gates
cannot be signed off from a Linux build.

`scripts/tasks/build-agent.sh` downloads Node 22.22.0 from nodejs.org, verifies
the pinned platform SHA256, and builds `BoBeAgent` with its lockfile. It stages
`build/agent-runtime/node` and `build/agent-runtime/agent/`, including all
production packages, pi assets, and checksum-pinned ripgrep/fd binaries with
their licenses. Dependency lifecycle scripts are disabled. The checksums and
supported Node archive targets are in `scripts/lib/agent-runtime.sh`.

For a manually launched development daemon, set `BOBE_NODE_PATH` to the staged
Node executable and `BOBE_AGENT_ENTRY` to the staged `agent/dist/main.js`.
The backend and app task scripts set these paths. Provider credentials belong
to the selected daemon authority; signing into another CLI does not sign BoBe
in. The app may restart its managed-local daemon when saved provider access
needs to be loaded. Attached-local and remote hosts are never restarted by the
app; their operator must restart them when requested.

## Common commands

| Command | Purpose |
|---|---|
| `scripts/tasks/check.sh` | Run the complete Rust, Swift, security, contract, and packaging validation gate |
| `scripts/tasks/run.sh` | Build and launch the debug app; set `BOBE_DEVELOPMENT_TEAM` to sign it |
| `scripts/tasks/backend.sh` | Run the daemon for Xcode development |
| `scripts/tasks/build-agent.sh` | Build and validate the pinned Node/pi distribution |
| `scripts/check-bodylink-contract.sh` | Check Rust/Swift/current SDK wire constants |
| `scripts/tasks/xcode.sh` | Regenerate the Xcode project |
| `scripts/tasks/clean.sh` | Remove Rust, Swift, Xcode and staged build output |

[Releasing](docs/releasing.md) describes the release scripts.

The [ESP32 SDK](esp32-sdk/README.md) is in `esp32-sdk/`. BodyLink contract checks
use it directly. `BOBE_ESP_SDK_ROOT` is an optional override for comparing
another SDK tree. Board development uses the SDK's own tools and local
dependencies. Building the daemon or Mac app does not require an ESP-IDF
installation.

Release and specialist operations live under `scripts/tasks/`; their checks live
directly under `scripts/`. Cargo manifests, the Swift package, and lint
configurations are the policy authorities.

`scripts/tasks/run.sh` without `BOBE_DEVELOPMENT_TEAM` builds an app with no
application identifier and no Keychain groups. It keeps its endpoint credential
in process memory, as [Signed or unsigned](docs/getting-started.md#signed-or-unsigned)
describes. A build that carries any signing identity must have the exact
endpoint group or credential operations fail. Work on the Keychain path, or on
a pairing that has to survive a relaunch, needs a signed build:
`BOBE_DEVELOPMENT_TEAM=<team-id> scripts/tasks/run.sh`.

## Engineering expectations

- Follow nearby code rather than generic language doctrine.
- Rust is an internal binary crate. Prefer crate-scoped visibility, structured
  errors, constructor injection, and the existing handler/service/repository
  boundaries.
- Swift uses `@Observable`, Swift concurrency, and AppKit only where native
  window or system behavior requires it.
- Source code contains no prose comments. Keep architectural, unsafe/FFI,
  lifecycle and protocol constraints in the current documentation. Preserve
  functional directives such as shebangs, Swift tools versions and linter rules.
- Wire changes must update Rust, Swift, tests, and cross-language constants
  together.
- Preserve loopback-safe defaults and daemon ownership of credentials and
  durable intelligence.

## Test scope

Keep tests that protect observable product behavior: authority isolation,
persistence and recovery, admission and cancellation, protocol compatibility,
privacy, and audio integrity. Combine repetitive input cases in table-driven
tests. Do not add tests that merely restate a default, assert incidental wording,
exercise a dependency's lock implementation, or construct values without an
assertion. A test-only helper should earn its maintenance cost by protecting a
real boundary, not make a trivial implementation look covered.

Test fixtures may fail fast with `unwrap`/`expect`; production builds retain
those Clippy warnings. Passing unit tests does not replace operating the daemon,
Mac surface, or physical body for the behavior being changed. The model-backed
STT corpus test is explicitly ignored by default; run it with
`BOBE_TEST_STT_MODEL_DIR` and `BOBE_TEST_STT_WAV` set using
`cargo test --manifest-path BoBeService/Cargo.toml real_streaming_model_transcribes_corpus -- --ignored`.
Native preference fixtures use `EphemeralUserDefaults` with absolute temporary
suite paths so CFPreferences never writes into the person's real preference
domain. Preserve that isolation when changing test helpers.

Full-product E2E scripts and frameworks are disposable agent inspection tools.
Keep their installation, scripts and artifacts in a temporary directory and
remove them after use. Do not add them to CI, builds or package/test tasks.
Use the real CLI and current SDK from a disposable inspection when runtime or
physical-body evidence is needed.

## Workflow

1. Inspect the current implementation and tests.
2. Make the smallest coherent change.
3. Run checks derived from the changed behavior, including runtime or UI evidence where applicable.
4. Explain user-visible behavior, schema and wire changes, dependency changes,
   and remaining uncertainty in the pull request.

Dependency changes need explicit review when they add proc macros, build scripts,
FFI or `-sys` crates, new registries, git sources, subprocesses, parsers,
archives, filesystem access, or network reach.

Release signing, notarization, Sparkle publication, and credentials are local
maintainer operations. [Releasing](docs/releasing.md) gives the procedure.
Never commit release secrets or generated key material.

## Distribution

Keep generated output, credentials, dependency caches and device-specific
recovery material out of distributed source.

Keep the project license and all applicable third-party license and attribution
notices in source and binary distributions. Provider artwork retains its own
brand terms; the project license does not replace them.

Contributions are licensed under the repository's MIT License and governed by
the [Code of Conduct](CODE_OF_CONDUCT.md).
