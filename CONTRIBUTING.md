# Contributing

Read [PRODUCT.md](PRODUCT.md) and [architecture](docs/architecture.md) first.
[AGENTS.md](AGENTS.md) has the repository rules. Bugs and ideas go to
[Issues](https://github.com/Sombrace/BoBe/issues); security reports follow
[SECURITY.md](SECURITY.md).

## Setup

You need macOS 26, Xcode 26 and [rustup](https://rustup.rs);
`rust-toolchain.toml` selects the Rust version.

```bash
git clone https://github.com/Sombrace/BoBe.git
cd BoBe
scripts/tasks/bootstrap-tools.sh
scripts/tasks/run.sh
```

Tools, Node and dependencies are pinned and downloaded into the checkout
(`.tools/`, `build/`); nothing is installed globally. Runtime testing needs a
provider key, a Rail Relay token, a subscription or Ollama.

`run.sh` builds an unsigned app unless you give it your team:
`BOBE_DEVELOPMENT_TEAM=<team-id> scripts/tasks/run.sh`. An unsigned app has no
Keychain access, so it keeps its endpoint credential in memory: managed-local
works normally, but an attached or remote pairing ends when the app quits.
Work on the Keychain path or on pairing across relaunches needs a signed
build. A source build is a different app to macOS than a release, so
permissions do not carry over; both use `~/.bobe/` unless `BOBE_DATA_DIR` is
set.

The daemon, worker and CLI also build on Linux with
`scripts/tasks/build-backend.sh debug|release`.

## Commands

| Command | Purpose |
|---|---|
| `scripts/tasks/check.sh` | The full gate on macOS: Rust, agent, Mac and iPhone apps, contracts, translations, dependencies, packaging |
| `scripts/tasks/run.sh` | Build and launch the debug app |
| `scripts/tasks/backend.sh` | Run a development daemon |
| `scripts/tasks/build-agent.sh` | Build and verify the pinned Node and pi worker |
| `scripts/tasks/xcode.sh` | Regenerate the Xcode project |
| `scripts/tasks/notices.sh` | Write the third-party notices releases ship |
| `scripts/tasks/clean.sh` | Remove build output |

`.github/workflows/ci.yml` runs the Linux half of `check.sh` when a
maintainer adds the `ci` label, on demand, and on `main`. Run `check.sh` on a
Mac for the Swift half. The [ESP32 SDK](esp32-sdk/README.md) has its own
tools; [Releasing](docs/releasing.md) covers release scripts.

## Conventions

- Follow nearby code. Rust is an internal binary crate with crate-scoped
  visibility, structured errors and handler, service and repository layers.
  Swift uses `@Observable`, Swift concurrency, and AppKit only where windows
  require it.
- Source code has no prose comments. Keep functional directives (shebangs,
  tool versions, lint rules); constraints belong in the docs.
- Change Rust, Swift, tests and `scripts/check-*` contracts together for any
  wire change.
- A schema change needs an upgrade step and test (see
  [architecture](docs/architecture.md#lifecycle)); a new UI string needs a
  translation in every locale, which `scripts/check-localization-catalogs.sh`
  enforces.
- Keep tests that protect behavior: authority, persistence and recovery,
  admission and cancellation, protocols, privacy and audio. Do not test
  defaults, wording or dependencies.
- Passing tests do not replace operating the daemon, app or body. End-to-end
  tools run from a temporary directory and are never committed.
- Dependencies that add build scripts, FFI, proc macros, subprocesses,
  parsers, archives, filesystem or network reach need explicit review.

## Pull requests

Describe the user-visible change, schema, wire and dependency changes, what
you ran, and what remains uncertain. Never commit secrets, keys, generated
output or caches. Contributions are MIT-licensed and follow the
[Code of Conduct](CODE_OF_CONDUCT.md); a distributed fork needs its own name
and logo ([TRADEMARKS.md](TRADEMARKS.md)).
