# BoBe agent worker

A private Node process that embeds the pi harness
(`@earendil-works/pi-coding-agent`) and runs pi sessions for the Rust daemon.
The daemon sends every path, provider, route, tool and limit, and owns
admission, identity, memory, goals, skills, secrets, voice and bodies.

## Build and test

```sh
scripts/tasks/build-agent.sh        # pinned Node, npm ci, build, tests, staging
```

Or, with Node 24.22 or later, `npm ci --ignore-scripts`, `npm run check`,
`npm test` and `npm run build` in this folder. Tests run the TypeScript
sources directly through Node's type stripping, so the sources use only
erasable syntax and `.ts` import specifiers; `npm run build` emits the shipped
`dist/main.js` without tests or fixtures. Tests use real pi sessions with
controlled provider adapters; they do not prove that a real provider works.

## Protocol

One JSON object per line on stdin and stdout (at most 16 MiB); diagnostics go
to stderr. Requests are `{id, method, params}`, replies `{id, result}` or
`{id, error}`, notifications `{method, params}`. Rust request IDs are positive,
worker callback IDs negative, and requests run concurrently. `src/protocol.ts`
defines every message, and `fixtures/host-contract-v1.json` is checked by the
worker and the Rust tests, so change both sides and the fixture together.

## Rules

- Rust sends every limit and budget; the worker adds no fallbacks. Only the
  subagent tool's limits live in `src/delegation.ts`.
- Shell, `rg`, `fd` and MCP processes run as Rust children through the
  `process` callback, never as Node children.
- Domain tools (memory, goals, schedules, owner questions, jobs, skills) call
  back into Rust; the worker keeps no domain state.
- No global pi auth, settings, prompts, extensions or project `.pi` files are
  loaded; sessions and sign-ins live under the data root's `agent/`.
- `assets/chat.md` is the only behavior text and the base system prompt.

`src/search.ts` adapts pi's grep and find to run under Rust's process
ownership; the adapted pi code is MIT-licensed and ships with
`assets/pi-search.LICENSE`. The `subagent` tool follows pi's MIT-licensed
subagent example.
