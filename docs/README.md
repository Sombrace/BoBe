# BoBe documentation

Start with the guides. The reference documents describe how BoBe is built and
are written for people changing the code.

## Guides

| Guide | Read it when |
|---|---|
| [Getting started](getting-started.md) | You want BoBe running on your Mac, from a release or from source |
| [Using BoBe](using-bobe.md) | You want to know what she can do and how to shape her |
| [Command line](cli.md) | You run BoBe without the app, on a server, or want to script her |
| [iPhone](iphone.md) | You want to pair a phone and understand what it supports |
| [Troubleshooting](troubleshooting.md) | Something does not start, connect, listen or speak |

## Reference

| Document | What it covers |
|---|---|
| [Product contract](../PRODUCT.md) | What BoBe is, and the product shapes it rejects |
| [Architecture](architecture.md) | How the Rust daemon, the pi worker and the Swift app are built |
| [Physical BoBe](physical-bobe.md) | Voice, embodiment and room devices |
| [ESP32 SDK](../esp32-sdk/README.md) | Building a body or another small surface |
| [Device pairing](device-pairing-crypto.md) | How a Mac and a daemon come to trust each other |
| [Updates](cli.md#updating) | App updates and signed standalone update files |
| [Releasing](releasing.md) | How maintainers build, sign, notarize and publish a release |

## Project

| Document | What it covers |
|---|---|
| [Contributing](../CONTRIBUTING.md) | Development setup, checks, conventions and review |
| [Security policy](../SECURITY.md) | Reporting a vulnerability, and what is in scope |
| [Code of conduct](../CODE_OF_CONDUCT.md) | How we treat each other |
| [License](../LICENSE) | MIT |

## How the pieces fit

The Mac application combines three programs:

- **The daemon** (`BoBeService/`, Rust) owns everything durable: identity,
  conversations, memory, goals, schedules, tools, credentials and policy. It is
  also the `bobe` command line.
- **The agent worker** (`BoBeAgent/`, TypeScript on a bundled Node) runs the pi
  agent harness that does the model and tool loop. The daemon starts it.
- **The Mac app** (`BoBeMacUI/`, Swift) is the face: the overlay, Settings,
  the microphone and the speaker. It does not hold BoBe's memory or decide
  what she does.

A Mac can run all three (the default), connect to a daemon already running on
the same Mac, or connect to a daemon on another computer. There is always one
BoBe, whichever surface you meet her through.

The [iPhone app](iphone.md) pairs with that runtime. It has its own conversation
thread and shares the same memory and goals. It does not run the daemon or agent.
