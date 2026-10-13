# Command line

The `bobe` command is BoBe's daemon and its control surface in one executable.
Use it to run BoBe without the Mac app, to keep her running on a server, or to
script her.

Every command here is the same BoBe the Mac app talks to. The command line is
a client of the daemon.

## Contents

- [What you need](#what-you-need)
- [Install](#install)
- [First run](#first-run)
- [Providers and models](#providers-and-models)
- [Talking and working](#talking-and-working)
- [Memory, profiles, skills and the workspace](#memory-profiles-skills-and-the-workspace)
- [Voice models](#voice-models)
- [Configuration](#configuration)
- [Remote access and pairing](#remote-access-and-pairing)
- [Backup and restore](#backup-and-restore)
- [Updating](#updating)
- [Command reference](#command-reference)

## What you need

| | |
|---|---|
| **A machine** | macOS 26 on Apple Silicon. The daemon also builds and runs on Linux x86_64; Ubuntu is not release-qualified, so treat Linux as a development target |
| **A model** | A provider API key, a Rail Relay token, a supported provider sign-in, or a local OpenAI-compatible endpoint such as Ollama |
| **To build** | [rustup](https://rustup.rs), Python 3.11 or later, and network access on the first build. Node is downloaded and pinned by the build; you do not install it |

## Install

Build a self-contained archive on the machine type you will run it on:

```bash
git clone https://github.com/Sombrace/BoBe.git
cd BoBe
scripts/tasks/standalone.sh
```

That produces `build/bobe-daemon-<version>-<platform>.tar.gz`, holding the
daemon, its pinned Node, the agent worker and an installer. Unpack it and run
the installer:

```bash
tar -xzf build/bobe-daemon-<version>-<platform>.tar.gz
./bobe-daemon-<version>-<platform>/install.sh
```

The installer copies BoBe to `~/.local/share/bobe/versions/<version>/` and
links `~/.local/bin/bobe`. Set `BOBE_INSTALL_PREFIX` to use a prefix other than
`~/.local`. Make sure the prefix's `bin` directory is on your `PATH`.

To remove it later, run the `uninstall.sh` from the same archive. It removes
the service and the installed files, and leaves your data in `~/.bobe/`.

For development you can skip the archive and run the daemon from the checkout
with `scripts/tasks/backend.sh`.

## First run

```bash
bobe setup
```

`bobe setup` starts BoBe as a service, asks how she should think (an online
AI or your own AI server, its key and a model), your name, and whether your
Mac and iPhone may reach her, then shows a pairing code. Each step it finds
done is skipped, so running it again only offers to pair another device. It
lists this computer's local network addresses first and tunnel addresses
such as Tailscale's after them; Docker bridges are left out. On Ubuntu with
ufw on, allow the port it prints. Everything below is what it does for you,
and what to use when you want to choose yourself.

| Command | What it does |
|---|---|
| `bobe setup` | Guided first run, described above |
| `bobe init` | Creates the data root, normally `~/.bobe/` |
| `bobe service install` | Installs and starts BoBe as a per-user service: a launchd agent on macOS (`com.bobe.daemon`), a systemd user unit on Linux (`bobe.service`) |
| `bobe status` | Shows whether the daemon is up and how far setup has got |
| `bobe wait-ready` | Waits until the daemon is ready; useful in scripts |
| `bobe doctor` | Checks the local runtime and configuration |
| `bobe service logs` | Shows the service's logs |
| `bobe service restart` | Restarts the service; needed after provider or model changes |
| `bobe service uninstall` | Stops and removes the service |

A service keeps running when no app is open and after you log back in, so BoBe
keeps reviewing goals and spending on a cloud provider. Stop it with
`bobe service uninstall` when you do not want that.

To run the daemon in the foreground instead, use `bobe serve`. It listens on
`127.0.0.1:8766` unless told otherwise.

## Providers and models

A provider profile says where inference runs and how BoBe authenticates. It
does not change who BoBe is.

### With an API key

```bash
bobe provider upsert openai --kind open-ai
printf '%s\n' "$OPENAI_API_KEY" | bobe provider credential openai
bobe model list --profile openai
bobe provider activate openai --kind open-ai \
  --chat-model <chat-model> \
  --goals-model <background-model> \
  --consolidate-model <background-model>
bobe service restart
```

The key is read from standard input so that it does not appear in your shell
history or the process list. It is stored in the daemon's own secret store
under the data root, readable only by your account.

Provider kinds: `open-ai`, `anthropic`, `deepseek`, `open-router`,
`rail-relay`, `copilot` and `open-ai-compatible`.

### With a provider sign-in

```bash
bobe provider upsert <profile> --kind <kind> --auth-method oauth
bobe provider auth login --profile <profile>
bobe provider auth status --profile <profile>
```

Sign-in state belongs to this BoBe. Signing in to another command line tool
does not sign BoBe in, and signing BoBe out does not touch that tool.

Anthropic is API-key-only: its
[published policy](https://code.claude.com/docs/en/legal-and-compliance#authentication-and-credential-use)
does not permit Claude subscription login in third-party apps. Rail Relay uses
the token and API base URL you supply, without a separate BoBe account.

### With local models

```bash
bobe provider activate local \
  --kind open-ai-compatible \
  --base-url http://127.0.0.1:11434/v1 \
  --local-runtime ollama \
  --offline \
  --chat-model qwen2.5:7b-instruct \
  --goals-model qwen2.5:7b-instruct \
  --consolidate-model qwen2.5:7b-instruct
bobe local-runtime install \
  --chat-model qwen2.5:7b-instruct \
  --batch-model qwen2.5:7b-instruct
bobe service restart
```

`bobe local-runtime install` finds or downloads Ollama and pulls the models.
An Ollama that BoBe started is not started again by a later daemon start, so
run Ollama as your own service if you want local inference to survive a
reboot. Continuous local inference is not a qualified path.

### Which model does which work

BoBe routes different kinds of work to different models:

| Workload | Used for |
|---|---|
| `chat` | Conversation |
| `goals` | Goal work and other background tasks |
| `small` | Quick decisions, summaries and screen context |
| `consolidate` | Tidying memory |

```bash
bobe model use <profile> <model-id> --workload small
bobe model use <profile> <model-id> --workload goals
bobe model use <profile> <model-id> --workload all
bobe provider list
bobe service restart
```

Profiles remember their choices. Provider, credential and route changes take
effect after a restart.

### Checking that it works

Activating a provider verifies a real streamed answer and a harmless tool
call. With a cloud provider this is billed like any other request.

`bobe doctor` checks local health and reports provider execution as untested.
`bobe doctor --probe` runs the same bounded check against the provider without
changing BoBe's conversation, memory or goals. Its JSON output separates model
execution, streaming and tool execution, and a failed check returns a nonzero
exit code.

### Finishing setup

```bash
printf '%s\n' 'Name and durable preferences for this BoBe.' |
  bobe profile create owner --stdin
bobe initialization advance personalization_required
bobe initialization advance ready
```

`bobe initialization status` shows where setup stands.

## Talking and working

```bash
bobe chat
bobe chat "Summarise what changed in my notes folder this week"
```

With no prompt, `bobe chat` is interactive. With a prompt, it sends one message
and prints the answer. Add `--json` for machine-readable output.

The command line has its own conversation thread. It shares memory, goals,
skills, tools and the workspace with the Mac app and any physical body, but the
transcripts stay separate. BoBe recalls an earlier conversation from another
thread when it is relevant.

### Runs

Messages can queue while BoBe is working, and separate threads can run at the
same time under shared limits.

```bash
bobe runs list
bobe runs stop <run-id>
bobe runs guide <run-id> "Use the staging database instead"
```

Guidance is applied at the next tool boundary. Add `--follow-up` to apply it
after the current task instead.

### Background jobs

BoBe can keep working on a bounded task after she has replied.

```bash
bobe jobs list
bobe jobs show <job-id>
bobe jobs cancel <job-id>
bobe jobs resume <job-id>
```

A tool effect that a restart interrupted is not replayed blindly. A goal review
can reconsider the task with its retained context and look at what already
happened; other jobs wait for an explicit resume.

### Schedules

```bash
bobe schedules list
bobe schedules create --at 2026-11-02T07:00:00Z \
  "Morning review" "Look at my calendar and tell me what needs preparing"
bobe schedules pause <schedule-id>
bobe schedules resume <schedule-id>
bobe schedules reschedule <schedule-id> --at 2026-11-03T07:00:00Z
bobe schedules cancel <schedule-id>
```

`--at` takes a timestamp with a time zone, such as `2026-11-02T07:00:00Z`. Add
`--every-seconds` for a repeating schedule, and `--timeout-seconds` or
`--max-turns` to limit how much work one run may do. Schedules and the hourly
goal review keep their due times across restarts. Missed intervals become one
catch-up, not a burst.

### Questions BoBe saves for you

When an important decision is missing, BoBe saves the exact question and
pauses that task without holding anything else up.

```bash
bobe input list
bobe input show <input-id>
bobe input answer <input-id> "Yes, send it"
bobe input cancel <input-id>
```

An answer resumes one continuation. A "yes" that was queued before the
question appeared cannot approve it. Routine failures and ordinary tool calls
do not ask for approval.

### The agent and its tools

```bash
bobe agent status --json
bobe agent compact
bobe agent tools list
bobe agent tools disable bash
bobe service restart
```

`agent status` shows the companion's actual context and the cache counters the
provider reports. `agent compact` asks for compaction while BoBe is idle; she
keeps the same conversation, memory and goals.

Tool selection applies to the companion, her helper agents and background jobs
after a restart. Disabling a tool is not a sandbox: other enabled shell or MCP
capabilities can still affect files.

## Memory, profiles, skills and the workspace

```bash
bobe memory show
bobe memory replace --file memory.md
bobe profile list
bobe profile create <name> --file notes.md
bobe profile disable <profile-id>
bobe skill list
bobe skill show <skill-id>
bobe workspace path
bobe workspace list
bobe workspace read <path>
```

Memory and goals can be turned off without deleting what is stored:

```bash
bobe config set memory.enabled false
bobe config set goals.enabled false
bobe service restart
```

With memory off it is no longer supplied to BoBe or tidied, and her memory
tools are removed. With goals off, goal reviews stop and her goal tools are
removed.

BoBe works in `workspace/` under the data root. Her file, search, shell, Git,
MCP, skill and delegation tools all start there, and work done by helper
agents is visible there too. Commands and paths carry your account's
permissions: this is a single-owner boundary, not an operating-system sandbox
or isolation between tenants.

BoBe can turn a procedure she has proved into a learned skill. There is no
approval queue for skills. `bobe skill reload` is a recovery command for skill
files you edited by hand.

## Voice models

`bobe model` selects language models. `bobe voice-models` installs the
daemon's speech models.

```bash
bobe voice-models status
bobe voice-models install
bobe voice-models install --kind tts
bobe voice-models cancel
```

Preview speech without a model-provider account:

```bash
bobe voice preview "Hello. This is BoBe." --output /tmp/bobe-preview.wav --play --json
```

Use `--voice marius`, `--speed 1.5`, or `--cancel-after 100ms` to exercise voice
selection, tempo and cancellation. Omit `--play` to save audio without playing it.
The output path must be new. Preview uses the daemon's real sentence processing,
speech engine and Opus stream, and does not create a conversation or call an LLM.
See [Pocket TTS](pocket-tts.md#preview-speech-without-a-provider-account) for setup
and the measurements reported by the command.

## Configuration

Everyday settings live in the Mac app. From the command line:

```bash
bobe config path
bobe config show
bobe config get voice.speed
bobe config set capture.enabled false
bobe config unset voice.speed
bobe config validate
```

Commonly used keys:

| Key | Meaning |
|---|---|
| `capture.enabled` | Screen awareness |
| `checkin.enabled`, `checkin.times` | Scheduled check-ins |
| `goals.enabled` | Goal reviews and goal tools |
| `memory.enabled` | Memory |
| `voice.enabled`, `voice.persona`, `voice.speed`, `voice.stt_language` | Voice |
| `mcp.enabled` | MCP tools |
| `server.host`, `server.port` | Where the daemon listens |

Settings for capture, check-ins, theme and voice apply at once. Provider
routes and execution limits need a restart.

Environment variables override configuration for advanced use, with a double
underscore between the section and the key:

```bash
BOBE_CAPTURE__ENABLED=false
BOBE_CAPTURE__INTERVAL_SECONDS=30
```

Provider profiles and model routes are managed with `bobe provider` and
`bobe model`, not with environment variables.

### Where data lives

Data is stored under `~/.bobe/` by default. Set `BOBE_DATA_DIR` to move the
whole data root. `BOBE_DATABASE__URL` is available as an explicit SQLite
override.

| Path | Holds |
|---|---|
| `config.toml` | Configuration |
| `data/` | The SQLite database: conversations, jobs, schedules, saved questions, paired devices |
| `goals/` | One Markdown file per goal |
| `memory.md` | Memory |
| `skills/` | Built-in and learned skills |
| `workspace/` | BoBe's working folder |
| `agent/` | Conversation transcripts and provider sign-in state |
| `models/` | Speech models |
| `backups/` | Backups you created |
| `logs/` | Service logs |
| `headless-secrets.json` | Provider API keys, readable only by your account |

Only one daemon can use a data root at a time. A second one refuses to start.

A privacy purge deletes conversations, memory, goals, learned skills,
workspace files and BoBe's session state. Provider credentials, installed
runtimes and backups you created are kept.

## Remote access and pairing

The daemon listens only on `127.0.0.1` until you deliberately configure remote
access.

On the machine that runs BoBe:

```bash
bobe config remote --host 0.0.0.0 --allowed-host bobe.example.net
bobe service restart
bobe device pair
```

`bobe device pair` shows a six-digit code that expires after five minutes. On
the Mac, choose **On another computer** in setup or in **Settings → Runtime**,
and enter the server address and the code.

- The daemon creates and renews its own TLS identity. There are no certificate
  files, fingerprints or shared tokens to handle. Pass `--tls-cert` and
  `--tls-key` only if you want to supply your own.
- Each paired Mac or phone gets its own credential. `bobe device list` shows
  them and `bobe device revoke` removes one, closing its live sessions.
- Remote traffic is always HTTPS. Plain HTTP is accepted only from a paired app
  on the same machine.
- Configuring remote access does not open your firewall or router. Making the
  machine reachable is up to you.
- A remote BoBe does not gain access to the files or shell of the Mac that
  shows her conversation. Her tools run on her own host. With screen awareness
  on, the attached Mac reads its own screen and sends her only the recognised
  text.

Most commands accept `--url` and `--token` to address another BoBe instead of
the local one. A Mac attached to a daemon it did not start never restarts that
daemon; whoever runs it does.

### Pairing a phone

On the machine that runs BoBe:

```bash
bobe device pair --kind phone
```

This prints a QR code for BoBe on the iPhone to scan, then the six-digit code
and BoBe's addresses to type in by hand if the camera cannot read it. The code
works once and expires after five minutes. `--label` names the phone; it
defaults to "iPhone".

The QR code holds a pairing link:

```text
bobe://pair?v=1&c=<code>&p=<key pin>&o=<host:port>&o=<host:port>
```

- `v=1` is the link format and `c` the six-digit code.
- `p` is the SHA-256 of the public key in BoBe's TLS certificate, the same
  digest the pairing exchange binds: 32 bytes in unpadded base64url, 43
  characters. The phone checks it on its first connection, before it sends
  anything.
- `o` repeats once per address, preferred first: each allowed host set with
  `bobe config remote`, with BoBe's port, then the address BoBe listens on
  when it is a specific network address. The phone tries them in order and
  always uses HTTPS. IPv6 hosts are bracketed, with the brackets written as
  `%5B` and `%5D`.

`--json` returns `code`, `expires_in_seconds`, `kind`, `label`, `pin`,
`origins` and `link` instead of the QR code. A Mac code (`--kind mac_app`, the
default) prints and returns only the code, its expiry, kind and label.

A phone can pair only with a BoBe it can reach. A BoBe that listens only on its
own computer refuses `--kind phone`, says how to turn on remote access, and
exits with status 2.

A paired Mac can make the same invitation over HTTP, and a paired phone or Mac
can revoke itself; [Deployment and trust](architecture.md#deployment-and-trust)
describes `POST /pairing/invitations` and `DELETE /endpoints/self`.

A paired phone can talk with BoBe by text and voice, answer her questions, read
and change her goals, memory, profiles and working folder, and see her jobs and
schedules. It cannot change providers, settings or configuration, manage other
devices, read a screen for her, steer a run in progress, manage jobs or
schedules, or erase her data. Each phone keeps its own thread.
`bobe device list` shows a phone with kind `other`, its label, and
`"scope_profile":"phone.v1"` among its capabilities; `bobe device revoke`
removes it and closes its live connections, as for a Mac.

Physical bodies and other small surfaces are enrolled with `bobe endpoint`.
See [Physical BoBe](physical-bobe.md) and the
[ESP32 SDK](../esp32-sdk/README.md).

## Backup and restore

```bash
bobe backup create
bobe backup list
bobe backup verify <path>
bobe backup restore <path>
```

A backup pauses the agent worker and includes BoBe's managed data, her thread
pointers and her conversation transcripts. Provider sign-in state is not put
in the backup. Restore verifies the backup first, replaces the managed data as
a whole with rollback if anything fails, and keeps the sign-in state of the
BoBe you restore into. A backup restores only into a BoBe of the same backup
format and database schema version; any other is rejected.

## Updating

The Mac app updates through its own **Check for Updates…** command.

A standalone daemon applies a signed update from three files:

```bash
bobe update apply \
  --manifest bobe-daemon-X.Y.Z-<platform>.tar.gz.manifest.json \
  --signature bobe-daemon-X.Y.Z-<platform>.tar.gz.manifest.json.sig \
  --archive bobe-daemon-X.Y.Z-<platform>.tar.gz
```

The daemon verifies the signature and the archive hash, checks that the new
version uses the same database schema version as your data, makes and verifies
a backup, and rolls back if the new version does not become ready. BoBe has no
database migrations: a release that changes the schema needs a new data
folder.
Use the manifest, signature and archive from the same release.
[Releasing](releasing.md) describes how maintainers build and sign them.

## Command reference

Run any command with `--help` for its options. Most commands accept `--json`.

| Command | Purpose |
|---|---|
| `bobe serve` | Run the daemon in the foreground |
| `bobe init` | Create the data root |
| `bobe status` | Daemon and setup state |
| `bobe wait-ready` | Wait until the daemon is ready |
| `bobe doctor` | Health checks; `--probe` also tests the provider |
| `bobe chat` | Talk to BoBe |
| `bobe runs` | List, stop or guide what BoBe is doing now |
| `bobe jobs` | Background tasks: list, show, start, cancel, resume |
| `bobe schedules` | Planned work: list, show, create, pause, resume, cancel, reschedule |
| `bobe input` | Questions BoBe saved for you: list, show, answer, cancel |
| `bobe agent` | Context status, compaction, and tool selection |
| `bobe config` | Read and change configuration; `remote` sets up remote access |
| `bobe initialization` | Setup state: status, advance |
| `bobe provider` | Provider profiles, credentials, routes and sign-in |
| `bobe model` | List models and assign them to workloads |
| `bobe local-runtime` | Install Ollama and local models |
| `bobe memory` | Show or replace memory |
| `bobe profile` | User profiles: list, create, enable, disable |
| `bobe skill` | Skills: list, show, reload |
| `bobe workspace` | BoBe's working folder: path, list, read, write |
| `bobe endpoint` | Physical bodies and other surfaces: enroll, list, revoke |
| `bobe device` | Paired Macs and phones: pair, list, revoke |
| `bobe voice-models` | Speech models: status, install, cancel |
| `bobe voice preview` | Generate, export and optionally play speech without a provider account |
| `bobe service` | The per-user service: install, status, logs, restart, uninstall |
| `bobe backup` | Create, list, verify and restore backups |
| `bobe update` | Apply a signed standalone update |
| `bobe version` | Version and compatibility information |
