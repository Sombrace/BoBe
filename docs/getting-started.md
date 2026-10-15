# Getting started

This guide takes you from nothing to a BoBe on your desktop. It covers what you
need, installing a release, building from source, and the first launch.

To run BoBe without the Mac app, see the [command line guide](cli.md).

## What you need

### To run BoBe

| | |
|---|---|
| **A Mac** | Apple Silicon, macOS 26 or later |
| **A model** | One of the options in [Choosing a model](#choosing-a-model) |
| **Disk space** | About 600 MB for the English speech models. Another language adds about 0.7 GB. Local models need several more gigabytes |
| **Network** | To download speech models and to reach a cloud provider, if you use one |

### To build BoBe from source

Everything above, plus:

| | |
|---|---|
| **Xcode 26 or later** | With Swift 6 and the command line tools |
| **rustup** | From [rustup.rs](https://rustup.rs). The repository's `rust-toolchain.toml` selects Rust 1.98.1 |
| **Network on the first build** | The build downloads pinned developer tools, Node 22.23.3, Swift packages and Rust crates. Downloaded tools and Node are checked against pinned hashes |

You do not need to install Node, npm, pi or any agent command line yourself.
The build downloads a pinned Node and keeps it inside the checkout.

### Choosing a model

BoBe does not include a model. You pick where her thinking runs:

| Option | What you need |
|---|---|
| OpenAI | An API key, or the provider's own sign-in |
| Anthropic | An API key. Claude subscription login is not offered in BoBe |
| DeepSeek, OpenRouter | An API key |
| Rail Relay | A token and its API address |
| GitHub Copilot | A GitHub account with Copilot |
| Local models | [Ollama](https://ollama.com), or any OpenAI-compatible endpoint. BoBe can install Ollama for you. Plan for 16 GB of memory or more |

An API key and a consumer subscription are different products with different
billing. Check what your provider account actually includes.

Cloud providers charge for what BoBe uses. She also thinks when you are not
talking to her: goal reviews run hourly by default, and check-ins run if you
turn them on. See [Keeping costs in hand](using-bobe.md#keeping-costs-in-hand).

## Install a release

1. Download the latest `BoBe.dmg` from the
   [Releases](https://github.com/Sombrace/BoBe/releases) page.
2. Open it and drag BoBe to Applications.
3. Open BoBe.

BoBe does not update herself in the background. Choose **Check for Updates…**
in Settings or from the menu bar icon when you want a newer version.

## Build from source

```bash
git clone https://github.com/Sombrace/BoBe.git
cd BoBe
scripts/tasks/bootstrap-tools.sh
scripts/tasks/run.sh
```

| Step | Result |
|---|---|
| `bootstrap-tools.sh` | Pinned developer tools in `.tools/`, inside the checkout |
| `run.sh` | Builds the agent worker, the daemon and the app, then starts the debug app |

Nothing is installed outside the checkout.

### Signed or unsigned

`run.sh` builds an unsigned app unless you give it a team. An unsigned app
cannot use the Keychain, so it keeps the credential for its BoBe in memory
only:

- **On this Mac** works the same. The app starts BoBe's daemon and gets a
  fresh credential from it at every launch.
- A connection to a BoBe you started yourself, or to one on another computer,
  lasts until you quit the app. After that you pair again.

To keep such a connection, or to work on the Keychain path, sign the build
with your own Apple development team:

```bash
BOBE_DEVELOPMENT_TEAM=<team-id> scripts/tasks/run.sh
```

`<team-id>` is your ten character Apple team identifier. The build uses
Xcode's automatic signing with the **Apple Development** identity, so that
team's development certificate and a Mac development profile need to be on
this Mac already. Releases are always signed.

A source build is a different app to macOS from a release. Permissions you
granted to one do not carry over to the other, and both use `~/.bobe/` unless
you set `BOBE_DATA_DIR`.

[CONTRIBUTING.md](../CONTRIBUTING.md) has the full development setup, the
check script and the conventions.

## First launch

Setup opens the first time you start BoBe. You can skip most steps and return
to them in Settings. If you connect to a BoBe that is already set up, setup
leaves out the steps she has already done, and its step count says so.

### 1. Where BoBe lives

| Choice | What it means |
|---|---|
| **On this Mac** | Recommended. The app starts BoBe's daemon and stops it when you quit |
| **Already running on this Mac** | Connect to a daemon you started yourself on this Mac, without starting another |
| **On another computer** | Connect to a BoBe you set up elsewhere. Run `bobe setup` there and choose Mac, or run `bobe device pair`, then enter the address and the six-digit code. The code expires after five minutes |

Her memory, goals and personality live in one place, whichever you choose.

### 2. How BoBe thinks

Choose **Use an online AI** or **Use an AI on your own computer**, then sign
in, paste a key or token, or let BoBe install Ollama and a model.

BoBe checks the model with a real streamed answer and a harmless tool call
before moving on. With a cloud provider this small check is billed like any
other request.

### 3. You

Tell BoBe your name, and a first goal if you have one in mind.

### 4. How present she is

| Mode | What it does |
|---|---|
| **Quiet** | No screen context and no scheduled check-ins. Goals keep working |
| **Balanced** | Uses occasional screen context, but does not start scheduled check-ins |
| **Proactive** | Checks screen context more often and can start scheduled check-ins |

### 5. Permissions

| Permission | Used for | Needed? |
|---|---|---|
| **Screen Recording** | Reading the text on your screen for context. The picture stays on the Mac | Only for screen awareness |
| **Microphone** | Talking to BoBe | Only for voice |

macOS may also ask whether BoBe can see which app you are using. She uses that
to name the app and window in screen context.

You can change any of these later in **System Settings → Privacy & Security**.

### 6. Voice

**Set up voice** downloads what she needs to hear you and to speak. You can
skip this and install it later from **Settings → Voice**.

### 7. A physical BoBe

Setup asks whether you have a physical BoBe. Choose **Not now** if you do not.

## After setup

BoBe sits on your desktop as a small face, and a BoBe icon appears in the menu
bar.

- **Click her face** to open the conversation and type.
- **Click the microphone** on her rim to talk.
- **Drag her face** to move her.
- **Click anywhere else** and the conversation folds away.

Quit from the menu bar icon. If the app started the daemon, the daemon stops
with it, and it also stops by itself if the app ever crashes. Nothing keeps
running without the menu bar icon unless you installed BoBe as a service from
the command line.

Continue with [Using BoBe](using-bobe.md).

## Where things are kept

| What | Where |
|---|---|
| Everything BoBe knows | `~/.bobe/` on the machine that runs her daemon |
| Goals | `~/.bobe/goals/`, one Markdown file each |
| Memory | `~/.bobe/memory.md` |
| Her working folder | `~/.bobe/workspace/` |
| Logs | `~/.bobe/logs/` |
| Speech models | `~/.bobe/models/` |
| Backups | `~/.bobe/backups/` |

Set `BOBE_DATA_DIR` before starting BoBe to keep all of this somewhere else.

## Removing BoBe

1. Quit BoBe from the menu bar.
2. Move BoBe from Applications to the Trash.
3. To remove her data as well, delete `~/.bobe/`. This cannot be undone, so
   make a [backup](cli.md#backup-and-restore) first if you might want her
   back.

If you installed the daemon as a service, run `bobe service uninstall` first.
