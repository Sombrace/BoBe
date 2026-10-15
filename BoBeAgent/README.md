# BoBe agent worker

A private Node process that embeds the pi harness
(`@earendil-works/pi-coding-agent`). The Rust daemon starts it, sends every
path, provider, route, tool and limit at initialization, and owns admission,
identity, memory, goals, skills, secrets, voice and bodies. The worker runs pi
sessions and nothing else.

## Build and test

```sh
scripts/tasks/build-agent.sh        # pinned Node, npm ci, build, tests, staging
```

Or, with Node 22.19 or later:

```sh
npm ci --prefix BoBeAgent --ignore-scripts --no-audit --no-fund
npm run build --prefix BoBeAgent
npm run check --prefix BoBeAgent
npm test --prefix BoBeAgent
```

The entry point is `dist/main.js`. Tests use real pi sessions with controlled
provider adapters; they are not evidence that a real provider works.

## Protocol

One JSON object per line on stdin and stdout (at most 16 MiB); stdout carries
only protocol frames and diagnostics go to stderr. Requests are
`{id, method, params}`, replies `{id, result}` or `{id, error}`, notifications
`{method, params}`. Rust request IDs are positive and worker callback IDs
negative; requests run concurrently.

`src/protocol.ts` defines every message. `fixtures/host-contract-v1.json` is
validated by both the worker tests and the Rust contract tests, so change both
sides and the fixture together.

## Rules

- Rust sends every execution limit and task budget; do not add worker-side
  fallbacks. Only the subagent tool's limits live in `src/delegation.ts`.
- Shell, `rg`, `fd` and MCP processes run as Rust children through the
  `process` callback, never as Node children.
- Domain tools (memory, goals, schedules, owner questions, background jobs,
  skills) call back into Rust; the worker keeps no domain state.
- No global pi auth, settings, prompts, extensions or project `.pi` files are
  loaded. Sessions, transcripts and sign-ins live under the data root's
  `agent/` directory.
- `assets/chat.md` is the only companion behavior text and the base system
  prompt. Rust sends persona (souls, profiles), memory and capability facts as
  data, and events as a marker plus data; `chat.md` says how to handle each.
- Anthropic is API-key-only.

`src/search.ts` adapts pi's grep and find so searches run under Rust's process
ownership; the adapted pi code is MIT-licensed and ships with
`assets/pi-search.LICENSE`. The `subagent` tool follows pi's MIT-licensed
subagent example with in-process sessions.
