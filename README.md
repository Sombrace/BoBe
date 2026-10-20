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
  <a href="https://github.com/Sombrace/BoBe/releases/latest"><b>Download</b></a>
  &nbsp;·&nbsp;
  <a href="docs/using-bobe.md"><b>Guide</b></a>
  &nbsp;·&nbsp;
  <a href="CONTRIBUTING.md"><b>Contributing</b></a>
</p>

<p align="center">
  <a href="https://github.com/Sombrace/BoBe/releases/latest"><img src="https://img.shields.io/github/v/release/Sombrace/BoBe?label=release&color=C67B5C" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-5B7350.svg" alt="MIT license"></a>
  <img src="https://img.shields.io/badge/platform-macOS%2026%2B-3B2F28?logo=apple&logoColor=white" alt="macOS 26+">
  <img src="https://img.shields.io/badge/arch-Apple%20Silicon-F0702A" alt="Apple Silicon">
</p>

<p align="center">
  <a href="https://www.bobebot.com"><img src="docs/media/bobe-film.gif" alt="BoBe's film: she wakes up, notices a speech you are writing late at night, remembers the wedding is on Saturday, turns it into a goal, works while you sleep, then jumps into a small yellow body" width="560"></a>
</p>

BoBe is a companion who lives on your Mac. She sits on your desktop as a small
face, notices what you are working on, remembers what matters to you, works on
your goals, and speaks up when she can help. You can type or talk to her, and
she can do real work with files, a shell, Git and your own tools.

She is yours: one BoBe, one memory, one set of goals, kept on your machine.
Her thinking runs where you choose: OpenAI, Anthropic, DeepSeek, OpenRouter,
Rail Relay, GitHub Copilot, or a local model through Ollama.

<p align="center">
  <img src="docs/media/app-conversation.png" alt="BoBe's conversation panel beside her avatar, drafting a wedding toast" width="720">
</p>

## What she does

- **Notices your work** by reading the text on your screen (optional); the
  picture never leaves your Mac.
- **Remembers you** in one memory document you can read and edit.
- **Works on your goals**, kept as Markdown files, with quiet progress every hour.
- **Speaks first, politely**: Quiet, Balanced or Proactive, with cooldowns.
- **Talks and listens** with speech on your own hardware; raw audio is never stored.
- **Uses tools**: files, search, shell, Git, MCP servers, skills and helper agents.

She also runs headless on a Mac or Linux machine, and the same BoBe can meet
you on an [iPhone](docs/using-bobe.md#iphone) or in a small
[ESP32 body](esp32-sdk/README.md) (a prototype).

## Install

Download `BoBe.dmg` from [Releases](https://github.com/Sombrace/BoBe/releases),
drag BoBe to Applications and open it. You need an Apple Silicon Mac with
macOS 26 and a model: an API key, a Rail Relay token, a supported sign-in or
Ollama ([guide](docs/using-bobe.md)).

To build from source (Xcode 27 and [rustup](https://rustup.rs); nothing is
installed globally):

```bash
git clone https://github.com/Sombrace/BoBe.git
cd BoBe
scripts/tasks/bootstrap-tools.sh
scripts/tasks/run.sh
```

Without the app, install the `bobe` command ([command line guide](docs/cli.md))
and run:

```bash
bobe setup
bobe chat
```

## Your data

Everything BoBe knows lives in `~/.bobe/` on the machine that runs her daemon,
with memory and goals in plain Markdown. A cloud model receives only the
context a request needs, and **Settings → Privacy** can delete her data. The
daemon listens only on `127.0.0.1` unless you turn on remote access, and her
tools run as your user account, which is not a sandbox. Report vulnerabilities
through [SECURITY.md](SECURITY.md).

## Documentation

| | |
|---|---|
| [Using BoBe](docs/using-bobe.md) | Install, setup, everyday use, iPhone, troubleshooting |
| [Command line](docs/cli.md) | Headless operation, providers, remote access, backup, updates |
| [Product](PRODUCT.md) | What BoBe is and is not |
| [Architecture](docs/architecture.md) | Which part owns what |
| [Speech](docs/speech.md) | Speech engines and model licenses |
| [Device pairing](docs/device-pairing-crypto.md) | How devices come to trust a daemon |
| [ESP32 SDK](esp32-sdk/README.md) | Building a physical body |
| [Releasing](docs/releasing.md) | Building, signing and publishing a release |
| [Contributing](CONTRIBUTING.md) | Development setup and conventions |

## License

The source code is [MIT](LICENSE). The BoBe name, logo, character and
promotional media are not; see [Trademarks](TRADEMARKS.md). Speech models
have their own licenses ([Speech](docs/speech.md#models-and-licenses)).

<p align="center">
  <br>
  <a href="https://www.bobebot.com"><img src="branding/logo-128.png" alt="BoBe" width="64"></a>
</p>
