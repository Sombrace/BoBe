<p align="center">
  <a href="https://www.bobebot.com">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="docs/media/banner-dark.png">
      <img src="docs/media/banner-light.png" alt="BoBe — notices, remembers what matters, helps before you ask" width="100%">
    </picture>
  </a>
</p>

<p align="center">
  <a href="https://www.bobebot.com"><b>Website</b></a>
  &nbsp;·&nbsp;
  <a href="https://github.com/athatheo/BoBe/releases/latest"><b>Download</b></a>
  &nbsp;·&nbsp;
  <a href="#quick-start"><b>Quick start</b></a>
  &nbsp;·&nbsp;
  <a href="docs/architecture.md"><b>Architecture</b></a>
  &nbsp;·&nbsp;
  <a href="CONTRIBUTING.md"><b>Contributing</b></a>
</p>

<p align="center">
  <a href="https://github.com/athatheo/BoBe/releases/latest"><img src="https://img.shields.io/github/v/release/athatheo/BoBe?label=release&color=C67B5C" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-5B7350.svg" alt="MIT license"></a>
  <img src="https://img.shields.io/badge/platform-macOS%2015%2B-3B2F28?logo=apple&logoColor=white" alt="macOS 15+">
  <img src="https://img.shields.io/badge/arch-Apple%20Silicon-F0702A" alt="Apple Silicon">
</p>

<p align="center">
  <a href="https://www.bobebot.com"><img src="docs/media/bobe-film.gif" alt="BoBe's film: she wakes up, notices a speech you are writing late at night, remembers the wedding is on Saturday, turns it into a goal, works while you sleep, then jumps into a small yellow body" width="560"></a>
</p>

<p align="center"><sub>BoBe's film, without its sound. The yellow body is a prototype.</sub></p>

## Meet BoBe

BoBe lives on your desktop as a transparent overlay. She watches what you're working on, builds memories over time, tracks your goals, and reaches out when she thinks she can help — like a thoughtful colleague who actually pays attention.

BoBe owns its data and runtime locally by default. You choose whether inference uses your GitHub Copilot subscription or a local Ollama model; only the context needed for a cloud answer leaves the Mac.

## What BoBe does

<table>
  <tr>
    <td width="33%" valign="top">
      <img src="docs/media/feature-notices.png" alt="BoBe looks at a document on the screen" width="100%">
      <h3>Notices your work</h3>
      Periodic screen captures, analyzed by a vision model to understand context. Screen awareness is optional.
    </td>
    <td width="33%" valign="top">
      <img src="docs/media/feature-remembers.png" alt="A note about a wedding on Saturday goes to BoBe" width="100%">
      <h3>Remembers you</h3>
      A local memory document. The agent can append durable facts through a bounded daemon tool.
    </td>
    <td width="33%" valign="top">
      <img src="docs/media/feature-goals.png" alt="A goal card: speech ready by Saturday" width="100%">
      <h3>Tracks your goals</h3>
      Human-readable goals, created after your confirmation, and used proactively while they are active.
    </td>
  </tr>
  <tr>
    <td width="33%" valign="top">
      <img src="docs/media/feature-quiet.png" alt="Night turns to morning while the window rests" width="100%">
      <h3>Respects your flow</h3>
      Cooldown logic and engagement awareness prevent interruptions.
    </td>
    <td width="33%" valign="top">
      <img src="docs/media/feature-speaks.png" alt="BoBe says good morning first" width="100%">
      <h3>Reaches out first</h3>
      A decision engine evaluates when help is valuable. BoBe is not a chatbot that waits for input.
    </td>
    <td width="33%" valign="top">
      <img src="docs/media/feature-soul.png" alt="A document named SOUL.md next to BoBe" width="100%">
      <h3>Has a soul you can edit</h3>
      Soul documents shape how BoBe communicates. They are plain text; rewrite them.
    </td>
  </tr>
</table>

**Uses tools.** Read-only file access, plus any [MCP](https://modelcontextprotocol.io/) server you connect.

## Make her yours

<p align="center">
  <img src="docs/media/themes.png" alt="BoBe in six colour themes" width="100%">
</p>

<p align="center">
  <sub>Terracotta Dreams &nbsp;·&nbsp; Soft Clay &nbsp;·&nbsp; Bubblegum &nbsp;·&nbsp; Cotton Candy &nbsp;·&nbsp; Midnight Clay &nbsp;·&nbsp; Twilight Rose</sub>
</p>

Six themes, light and dark, or follow the system. Choose one under **Settings → Appearance**.

## A body, in the lab

<table>
  <tr>
    <td width="42%" valign="top">
      <img src="docs/media/feature-body.png" alt="A photo of a small yellow toy with BoBe's face on its screen, held in two hands" width="100%">
    </td>
    <td valign="top">
      <p>BoBe does not have to live only on a screen. The small yellow body in the film is real hardware: an ESP32 toy that shows her face and talks to the same companion on your Mac. One BoBe, one memory, more than one place to meet her.</p>
      <p>It is a prototype. Mobile and physical-device surfaces are platform directions, not shipped applications.</p>
      <p>
        <a href="docs/physical-bobe.md">Physical BoBe</a>
        &nbsp;·&nbsp;
        <a href="docs/platform-architecture.md">Platform architecture</a>
      </p>
    </td>
  </tr>
</table>

## Quick start

### Download

Get the latest `BoBe.dmg` from [Releases](https://github.com/athatheo/BoBe/releases), drag BoBe to Applications, and launch it. More help is on [bobebot.com](https://www.bobebot.com).

### Build from source

```bash
git clone https://github.com/athatheo/BoBe.git
cd BoBe
just run
```

> Requires macOS 15+, Rust 1.97.1+, Xcode 16+, and [just](https://github.com/casey/just). See [CONTRIBUTING.md](CONTRIBUTING.md) for full prerequisites.

### First launch

BoBe's eight-stage setup wizard walks you through:

1. **Choose your AI** — GitHub Copilot cloud or local Ollama
2. **Authenticate or install** — sign in through BoBe's signed Copilot CLI helper, or let BoBe install Ollama and models
3. **Personalize** — tell BoBe your name and optionally create a first goal
4. **Choose proactivity** — tune how often BoBe observes and reaches out
5. **Grant permissions** — screen awareness is optional
6. **Prepare voice** — install the speech models used by the Mac

After setup, BoBe appears as a floating overlay on your desktop with a menu bar icon.

## Models

| Engine | Description |
|--------|-------------|
| **GitHub Copilot** | Cloud inference using your Copilot subscription and CLI authentication. |
| **[Ollama](https://ollama.ai)** | Local inference. BoBe can install the runtime and selected models. |

## Supported platforms

| Platform | Architecture | Minimum version | Status |
|----------|--------------|-----------------|--------|
| macOS | Apple Silicon (arm64) | macOS 15 Sequoia | ✅ Supported |
| Ubuntu daemon | x86_64 | — | 🧭 Compatibility target; no packaged release or continuous validation |

## Configuration

Everything is in BoBe's settings panel (menu bar icon → Settings). Settings persist to `~/.bobe/config.toml`.

<details>
<summary>Environment overrides and data location</summary>

<br>

```bash
BOBE_ENGINE__ENGINE=local
BOBE_CAPTURE__ENABLED=false
BOBE_CAPTURE__INTERVAL_SECONDS=30
```

Data is stored under `~/.bobe/` by default. Set `BOBE_DATA_DIR` to relocate the canonical data root; `BOBE_DATABASE__URL` remains available as an explicit SQLite override.

</details>

## Privacy and security

BoBe handles sensitive data: screen captures, personal context, and integration secrets. Report a vulnerability through [SECURITY.md](SECURITY.md).

- **Loopback by default** — the daemon binds to `127.0.0.1` unless remote mode is deliberately configured
- **Secrets** stored in macOS Keychain and handled in-memory with the `secrecy` crate
- **Arbitrary file writes and shell commands** are denied; memory and goal mutations use bounded daemon-owned tools
- **MCP commands** validated against a configurable blocklist
- **CORS** locked to localhost origins
- **Remote or reverse-proxy access** must configure an API bearer token and a TLS certificate and key; adding a public allowed host without both is rejected at startup, including when the daemon itself binds to loopback behind a proxy

## Documentation

| Document | What it covers |
|---|---|
| [Architecture](docs/architecture.md) | How the Rust daemon and the Swift app are built today |
| [Platform architecture](docs/platform-architecture.md) | Where BoBe runs, and where it is going |
| [Physical BoBe](docs/physical-bobe.md) | Voice, embodiment, and room devices |
| [Updating over the air](docs/UpdatingOTA.md) | Sparkle and the local release runbook |
| [Rust guidelines](docs/RUST_GUIDELINES.md) | How the daemon's code is written |
| [Contributing](CONTRIBUTING.md) | Setup, conventions, and project structure |

## Contributing

Contributions are welcome. [CONTRIBUTING.md](CONTRIBUTING.md) has the development setup, architecture, coding conventions, and the full project structure. This project follows the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md).

## License

[MIT](LICENSE)

<p align="center">
  <br>
  <a href="https://www.bobebot.com"><img src="branding/logo-128.png" alt="BoBe" width="64"></a>
  <br>
  <sub><a href="https://www.bobebot.com">bobebot.com</a></sub>
</p>
