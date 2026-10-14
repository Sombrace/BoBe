# Troubleshooting

Start here when BoBe does not start, connect, listen or speak.

## First checks

| Check | How |
|---|---|
| Is the daemon up, and how far did setup get? | `bobe status` |
| Is the local runtime healthy? | `bobe doctor` |
| Can the provider actually answer? | `bobe doctor --probe`. This makes a small real request, which a cloud provider bills |
| What did the daemon log? | `~/.bobe/logs/`, or `bobe service logs` for an installed service |
| Which version is this? | `bobe version` |

The `bobe` command is inside the app at
`/Applications/BoBe.app/Contents/MacOS/bobe-daemon` if you have not installed
it separately.

## The app

### "BoBe couldn't start her backend"

The app could not start its daemon. Try again once; a first start after an
update can be slow.

If it keeps happening:

- **Another BoBe is using the same data.** Only one daemon can use `~/.bobe/`
  at a time. If you installed BoBe as a service with `bobe service install`,
  that daemon already holds it. Either choose **Already running on this Mac**
  in **Settings → Runtime**, or remove the service with
  `bobe service uninstall`.
- **Something else is using port 8766.** Quit the other program, or stop a
  stray BoBe daemon.
- **Read the log** in `~/.bobe/logs/`. The alert also shows the daemon's last
  output.
- **Reinstall** from the original DMG if nothing above helps. Your data in
  `~/.bobe/` is kept.

### BoBe is not on screen

Click the menu bar icon and choose **Show BoBe**. If there is no menu bar icon,
BoBe is not running.

### Quit and nothing is left running

Quitting from the menu bar stops the daemon the app started. If a `bobe`
process is still there afterwards, it is a service or a daemon you started
yourself:

```bash
bobe service status
bobe service uninstall
```

## Models and providers

### The provider rejects the key or sign-in

- Paste the key again in **Settings → AI & tools**, or pipe it to
  `bobe provider credential <profile>`.
- An API key and a consumer subscription are different products. A
  subscription does not give you an API key, and the reverse.
- Anthropic works with an API key only. Claude subscription login is not
  offered in BoBe.
- Run `bobe doctor --probe` to see whether model execution, streaming or tool
  use is the part that fails.

### A model change did not take effect

Provider, credential and model changes apply after a restart. The app restarts
a daemon it started itself. For a service, run `bobe service restart`. A Mac
connected to a BoBe on another computer cannot restart it; do that on the
other computer.

### Local models stopped working after a reboot

An Ollama that BoBe started during installation is not started again later.
Start Ollama yourself, or run it as your own login service, then check with
`bobe doctor --probe`.

### BoBe is spending more than expected

She thinks on her own: goal reviews, check-ins and screen context. See
[Keeping costs in hand](using-bobe.md#keeping-costs-in-hand). The quickest
stop is to quit the app, or `bobe service uninstall` for a service.

## Voice

### The speech models did not install

Retry from **Settings → Voice**. The download is about 600 MB for English and
needs a steady connection. If it fails again, the reason is in
`~/.bobe/logs/`. A download whose checksum does not match is refused.

### BoBe does not hear me

- Check **System Settings → Privacy & Security → Microphone** and allow BoBe.
- Check that **Enable voice** is on in **Settings → Voice** and that the
  listener for your language shows **Installed**.
- Check the input device in **System Settings → Sound**.
- Voice can be in use by one place at a time. If a physical body or another
  session is in a voice conversation, finish that first.

### BoBe answers before I finish, or waits too long

Change **Pause sensitivity** in **Settings → Voice**. **Patient** waits longer
after you stop talking; **Tight** answers sooner.

### BoBe does not stop when I talk over her

Short acknowledgements such as "yeah" or "mm-hmm" are meant not to stop her.
Say "wait" or "stop", or anything two words long. The word lists are English;
in other languages she counts words instead.

### Recognition is poor in my language

English is the most reliable. Other languages are still maturing.

## Screen awareness

### BoBe does not seem to know what is on my screen

- Screen awareness is off in **Quiet** mode. Choose **Balanced** or
  **Proactive** in **Settings → Behavior**, or turn on **Screen Capture**
  there.
- Allow BoBe in **System Settings → Privacy & Security → Screen Recording**,
  then quit and reopen her.
- She reads text only. Pictures, video and drawings without text are not
  described.
- Only the main display is read.

## Connecting to a BoBe on another computer

| Message | What to do |
|---|---|
| The pairing code is rejected | Codes expire after five minutes. Run `bobe device pair` again on the server |
| BoBe couldn't be reached | Check the address and port, that the server's firewall allows the connection, and that the host name you are using was given to `bobe config remote --allowed-host` |
| That address answered as a different BoBe | The server's identity is not the one this Mac paired with. Pair again from the server, and only if you expected the change |

Remote access has to be turned on deliberately on the server. See
[Remote access and pairing](cli.md#remote-access-and-pairing).

## Command line

### `bobe: command not found`

The installer links `~/.local/bin/bobe`. Add `~/.local/bin` to your `PATH`.

### The daemon started from a checkout cannot run the agent

A daemon you start by hand needs to know where the agent worker is. Set
`BOBE_NODE_PATH` to `build/agent-runtime/node` and `BOBE_AGENT_ENTRY` to
`build/agent-runtime/agent/dist/main.js`, or use `scripts/tasks/backend.sh`,
which sets both.

### The daemon refuses the database

`database schema version N is not the version this daemon uses` means the data
in `~/.bobe/` was written by a different build. BoBe has no database
migrations, so a build with a changed schema cannot open older data. Run
the build that wrote it, or move `~/.bobe/` aside and set BoBe up again.

### A second daemon refuses to start

Only one daemon can use a data root. Stop the first one, or give the second
its own data with `BOBE_DATA_DIR`.

## Asking for help

Open an [issue](https://github.com/Sombrace/BoBe/issues) with:

- what you did, what you expected, and what happened
- the output of `bobe version` and `bobe doctor`
- your macOS version, and whether you installed a release or built from source
- the relevant part of the log

Read the log before you paste it and remove anything private. Do not post API
keys, tokens or pairing codes. Report security problems privately, as described
in [SECURITY.md](../SECURITY.md).
