# Using BoBe

Installing the Mac app, setting it up, everyday use and common problems. To run
BoBe without the app, see the [command line guide](cli.md).

## Install

You need an Apple Silicon Mac with macOS 26 and a model: an API key (OpenAI,
Anthropic, DeepSeek, OpenRouter), a Rail Relay token, a supported sign-in
(OpenAI, GitHub Copilot), or [Ollama](https://ollama.com) with 16 GB of memory
or more. Voice needs about 600 MB of speech models for English.

Download `BoBe.dmg` from [Releases](https://github.com/Sombrace/BoBe/releases),
drag BoBe to Applications and open it. **Check for Updates…** in the menu bar
icon or Settings installs new versions.

## First launch

Setup asks only what BoBe cannot decide; Settings changes any of it later.

1. **Where BoBe lives:** *On this Mac* (recommended; the app starts and stops
   her), *Already running on this Mac*, or *On another computer* (run
   `bobe device pair` there and enter its address and code).
2. **How she thinks:** an online AI with a sign-in, key or token, or a local
   model. BoBe checks it with a small real request, which a cloud provider bills.
3. **You:** your name, and a first goal if you like.
4. **How present she is:** Quiet, Balanced or Proactive.
5. **Permissions:** Screen Recording for screen awareness, Microphone for voice.
6. **Voice:** downloads the speech models.

## Everyday use

<p align="center">
  <img src="media/app-conversation.png" alt="BoBe's conversation panel beside her avatar" width="720">
</p>

Click her face to open or close the conversation, drag it to move her, and
press Escape or click elsewhere to fold the conversation away. Point near her
for the chat, microphone and Settings controls. While she works her rim glows
and the send button becomes **guide** or **stop**. The menu bar icon shows or
hides her, opens Settings and quits; quitting also stops a daemon the app
started.

**Voice.** Click the microphone and speak; she answers out loud. Speech is
recognized on your Mac and raw audio is never stored. Talk over her to
interrupt: "yeah" or "mm-hmm" does not stop her, "wait", "no" or anything two
words long does. **Settings → Voice** sets the language, pause sensitivity,
voice and speed; English is the most reliable.

**Speaking first.** Check-ins, things she noticed and finished work arrive as
small bubbles; missed ones wait with a count on her face. **Settings →
Behavior** chooses Quiet (no screen context or check-ins), Balanced (occasional
screen context) or Proactive (more screen context and check-ins). Goals keep
working in every mode.

**Goals** are Markdown files in `~/.bobe/goals/`. Create one in **Settings →
Goals** or tell her. She reviews active goals hourly, does quiet work, decides
separately whether to tell you, and completes or pauses goals herself. Goals
whose reviews change nothing three times in a row rest until they change.

**Memory, profiles and souls.** Memory (**Settings → Memories**,
`~/.bobe/memory.md`) holds lasting facts; user profiles say who you are and how
you like things done; souls set her personality. Each place you meet her (the
app, the command line, a phone, a body) keeps its own conversation, and all
share the same memory and goals.

**Screen awareness.** About every two minutes the Mac reads the text on the
main display, discards the picture and sends only the text (up to 6,000
characters), the app name and the window title to your small model. It needs
Screen Recording and is off in Quiet mode.

**Tools.** She reads and writes files, searches, runs shell commands and Git,
uses MCP servers you connect, follows and writes skills, and delegates to
helper agents, all in `~/.bobe/workspace/`. **This is not a sandbox:** her tools
run as your user account. She can keep working after replying, work on a
schedule, and save an important question for you instead of guessing.

**Privacy and cost.** Everything lives in `~/.bobe/` on the machine that runs
her daemon; a cloud model receives the context of each request. **Settings →
Privacy** shows where each kind of work goes and can delete her data. Goal
reviews, screen context and check-ins also cost money on a cloud provider; to
spend less, choose Quiet, pause goals, give small work to a cheap model in
**Settings → AI & tools**, or use a local model.

## iPhone

The iPhone app is another surface of the same BoBe: its own conversation,
shared memory and goals, hold-to-talk recognized on the phone, and owner
questions. It needs a BoBe reachable on the network. Pair it from **Settings →
Devices → Pair iPhone** on the Mac, or `bobe device pair --kind phone` on a
standalone daemon, and scan the QR code. The phone cannot change providers or
settings, manage devices or erase data, and shows replies as text. Build it
from `BoBeiOS/project.yml` after `scripts/tasks/build-pairing-ios.sh`.

## Troubleshooting

| Problem | What to do |
|---|---|
| "BoBe couldn't start her backend" | Another BoBe holds `~/.bobe/` (choose *Already running on this Mac*, or `bobe service uninstall`), or port 8766 is taken; logs are in `~/.bobe/logs/` |
| She is not on screen | Menu bar icon → **Show BoBe** |
| The provider rejects the key | Paste it again in **Settings → AI & tools**; `bobe doctor --probe` shows which part fails |
| A model change did nothing | Changes apply after a restart; the app restarts only a daemon it started |
| Local models stopped | BoBe stops its Ollama with the daemon; reinstall the local runtime or run Ollama yourself |
| She does not hear you | Allow the microphone in System Settings and check **Settings → Voice** |
| She answers too soon or too late | Change **Pause sensitivity** |
| She does not see your screen | Leave Quiet mode, allow Screen Recording, reopen BoBe |
| A pairing code is rejected | Codes expire after five minutes; run `bobe device pair` again |
| Another computer cannot be reached | Check the address, the firewall and `bobe config remote --allowed-host` |

The `bobe` command inside the app is
`/Applications/BoBe.app/Contents/MacOS/bobe-daemon`. A bug report should
include `bobe version`, `bobe doctor` and the relevant log with keys, tokens
and pairing codes removed; security problems go to [SECURITY.md](../SECURITY.md).

To remove BoBe, quit from the menu bar, run `bobe service uninstall` if you
installed a service, and move the app to the Trash. Delete `~/.bobe/` to remove
her data, after `bobe backup create` if you might want her back.
