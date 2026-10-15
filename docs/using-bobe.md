# Using BoBe

How to install the Mac app, set it up, use it day to day, and fix common
problems. To run BoBe without the app, see the [command line guide](cli.md).

## Install

You need an Apple Silicon Mac with macOS 26 and a model: an API key (OpenAI,
Anthropic, DeepSeek, OpenRouter), a Rail Relay token, a supported sign-in
(OpenAI, GitHub Copilot), or [Ollama](https://ollama.com) with 16 GB of memory
or more. Voice needs about 600 MB of speech models for English.

Download `BoBe.dmg` from [Releases](https://github.com/Sombrace/BoBe/releases),
drag BoBe to Applications and open it. Updates come from **Check for
Updates…** in the menu bar icon or Settings.

To build from source instead, see [Contributing](../CONTRIBUTING.md#setup).

## First launch

Setup asks only what BoBe cannot decide; every step can be changed later in
Settings.

1. **Where BoBe lives.** *On this Mac* (recommended: the app starts BoBe and
   stops her when you quit), *Already running on this Mac*, or *On another
   computer* (run `bobe device pair` there and enter the address and code).
2. **How she thinks.** An online AI with a sign-in, key or token, or a model on
   your own computer. BoBe checks it with a small real request, which a cloud
   provider bills.
3. **You.** Your name, and a first goal if you like.
4. **How present she is.** Quiet, Balanced or Proactive.
5. **Permissions.** Screen Recording only for screen awareness; Microphone
   only for voice.
6. **Voice.** Downloads the speech models.

## Everyday use

<p align="center">
  <img src="media/app-conversation.png" alt="BoBe's conversation panel beside her avatar" width="720">
</p>

| You | BoBe |
|---|---|
| Click her face | Opens the conversation; click again to close it |
| Point near her | Shows the chat, microphone and Settings controls |
| Drag her face | Moves her |
| Click elsewhere | Folds the conversation away unless you are writing |
| Press Escape | Closes the conversation |

While she works, her rim glows and the send button becomes **guide** or
**stop**. The menu bar icon shows or hides her, opens Settings and quits. Quit
there and a daemon the app started stops too.

### Talking to BoBe

Click the microphone and speak; she answers out loud. Speech is recognized on
your Mac and her voice runs where her daemon runs; raw audio is never stored.
Talk over her to interrupt: "yeah" or "mm-hmm" does not stop her, "wait" or
"no" does, and so does anything two words long. If you trail off on "and",
she waits for the rest. **Settings → Voice** sets the language, pause
sensitivity, voice and speed. English is the most reliable language.

### When BoBe speaks first

Check-ins, things she noticed and finished work arrive as small bubbles with
one-tap replies. Ones you missed wait, and her face shows a count. **Settings →
Behavior** chooses how present she is:

| Mode | Screen context | Check-ins | Goals |
|---|---|---|---|
| Quiet | Off | Off | Keep working |
| Balanced | Occasional | Off | Keep working |
| Proactive | More often | On | Keep working |

### Goals

A goal is a Markdown file in `~/.bobe/goals/`. Create one in **Settings →
Goals** or just tell her. She reviews active goals every hour, does quiet work,
and decides separately whether to tell you now, later or not at all. She
completes a goal when it is done and pauses it when it cannot go on. Three
reviews in a row that change nothing rest the goal until it changes. You can
pause, complete or archive any goal.

### Memory, profiles and souls

| | What it is | Where |
|---|---|---|
| Memory | Lasting facts about you and your work | **Settings → Memories**, `~/.bobe/memory.md` |
| User profiles | Who you are and how you like things done | **Settings → User Profiles** |
| Souls | Her personality | **Settings → Souls** |

Each place you meet her (the app, the command line, a phone, a body) keeps its
own conversation, and all share the same memory and goals.

### Screen awareness

When on, about every two minutes the Mac reads the text on the main display,
discards the picture, and sends only the text (at most 6,000 characters), the
app name and the window title to your small model. Pictures without text are
not seen. It needs Screen Recording permission and is off in Quiet mode.

### Tools and work

She reads and writes files, searches, runs shell commands and Git, uses MCP
servers you connect, follows and writes skills, and hands work to helper
agents, all in `~/.bobe/workspace/`. **This is not a sandbox:** her tools run
as your user account. She can continue a task after replying, work on a
schedule, and save an important question for you instead of guessing.

### Privacy and cost

Everything lives in `~/.bobe/` on the machine that runs her daemon. A cloud
model receives the context for each request; **Settings → Privacy** shows
where each kind of work goes and can delete conversations, memory, goals and
work. Cloud providers bill for goal reviews, screen context and check-ins too.
To spend less, choose Quiet, pause goals you are not working on, give the
small work to a cheap model in **Settings → AI & tools**, or use a local
model.

## iPhone

The iPhone app is another surface of the same BoBe: its own conversation,
the shared memory and goals, hold-to-talk speech recognized on the phone, and
owner questions. It needs a BoBe reachable on the network. On the Mac, open
**Settings → Devices → Pair iPhone**; on a standalone daemon run
`bobe device pair --kind phone`. Scan the QR code or enter the address and
code. The phone cannot change providers or settings, manage devices or erase
data, and it does not play spoken replies. Build it from `BoBeiOS/project.yml`
after `scripts/tasks/build-pairing-ios.sh`.

## Troubleshooting

| Problem | What to do |
|---|---|
| "BoBe couldn't start her backend" | Another BoBe may hold `~/.bobe/` (a service from `bobe service install`: choose *Already running on this Mac* or run `bobe service uninstall`), or port 8766 is taken. The log is in `~/.bobe/logs/` |
| She is not on screen | Menu bar icon → **Show BoBe** |
| The provider rejects the key | Paste it again in **Settings → AI & tools**; an API key and a subscription are different products; `bobe doctor --probe` shows which part fails |
| A model change did nothing | Changes apply after a restart; the app restarts only a daemon it started |
| Local models stop after a reboot | Start Ollama yourself; BoBe does not restart an Ollama it installed |
| She does not hear you | Allow the microphone in System Settings, check **Enable voice** and the installed listener in **Settings → Voice**, and finish any voice conversation on another device |
| She answers too soon or too late | Change **Pause sensitivity** |
| She does not see your screen | Leave Quiet mode, allow Screen Recording, reopen BoBe |
| A pairing code is rejected | Codes expire after five minutes; run `bobe device pair` again |
| Another computer cannot be reached | Check the address, the firewall and `bobe config remote --allowed-host` |

The `bobe` command inside the app is
`/Applications/BoBe.app/Contents/MacOS/bobe-daemon`. When reporting a bug,
include `bobe version`, `bobe doctor` and the relevant log, with keys, tokens
and pairing codes removed. Security problems go to [SECURITY.md](../SECURITY.md).

## Removing BoBe

Quit from the menu bar, run `bobe service uninstall` if you installed a
service, and move BoBe to the Trash. Delete `~/.bobe/` to remove her data;
make a backup first with `bobe backup create` if you might want her back.
