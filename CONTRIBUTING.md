# Contributing

Read [PRODUCT.md](PRODUCT.md) and the [architecture](docs/architecture.md)
first; [AGENTS.md](AGENTS.md) has the repository rules. Bugs and ideas go to
[Issues](https://github.com/Sombrace/BoBe/issues) and security reports to
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

`run.sh` builds an unsigned app unless you set
`BOBE_DEVELOPMENT_TEAM=<team-id>`. An unsigned app keeps its endpoint
credential in memory, so attached and remote pairings end when it quits. A
source build is a different app to macOS than a release, so permissions do not
carry over; both use `~/.bobe/` unless `BOBE_DATA_DIR` is set. The daemon,
worker and CLI also build on Linux with `scripts/tasks/build-backend.sh`.

## Commands

| Command | Purpose |
|---|---|
| `scripts/tasks/check.sh` | The full gate on macOS: Rust, worker, Mac and iPhone apps, contracts, translations, dependencies, packaging |
| `scripts/tasks/run.sh` | Build and launch the debug app |
| `scripts/tasks/backend.sh` | Run a development daemon |
| `scripts/tasks/build-agent.sh` | Build and verify the pinned Node and pi worker |
| `scripts/tasks/xcode.sh` | Regenerate the Xcode project |
| `scripts/tasks/clean.sh` | Remove build output |

`.github/workflows/ci.yml` runs the Linux half of `check.sh` on demand, on
`main`, and on pull requests labeled `ci`. The [ESP32 SDK](esp32-sdk/README.md)
has its own tools, and [Releasing](docs/releasing.md) covers release scripts.

## Conventions

- Follow nearby code. Rust is a binary crate with crate-scoped visibility,
  structured errors, and handler, service and repository layers. Swift uses
  `@Observable` and Swift concurrency, with AppKit only where windows need it.
- Source code has no prose comments; keep functional directives.
- Change Rust, Swift, tests and `scripts/check-*` contracts together for any
  wire change. A schema change needs an upgrade step and test; a new UI string
  needs every translation.
- Keep tests that protect behavior (authority, persistence, recovery,
  admission, cancellation, protocols, privacy, audio), not tests of defaults,
  wording or dependencies. Passing tests do not replace operating the daemon,
  app or body.
- Dependencies with build scripts, FFI, proc macros, subprocesses, parsers,
  archives, filesystem or network reach need explicit review.

## Pull requests

Describe the user-visible change; schema, wire and dependency changes; what
you ran; and what remains uncertain. Never commit secrets, keys, generated
output or caches. Contributions are MIT-licensed and follow the
[Code of Conduct](CODE_OF_CONDUCT.md); a distributed fork needs its own name
and logo ([TRADEMARKS.md](TRADEMARKS.md)).
