# Command line

`bobe` is both the daemon and its control client. Use it to run BoBe without
the Mac app, on a server, or from scripts. Run any command with `--help`; most
accept `--json`, and `--url` and `--token` to address another BoBe.

## Install

| Machine | How |
|---|---|
| Mac with the app | **Settings → Runtime → Install Command Line Tool…** links `~/.local/bin/bobe` to the app's own copy |
| Linux x86_64 | Download `bobe-daemon-<version>-linux-x86_64.tar.gz` and its `.sha256` from [Releases](https://github.com/Sombrace/BoBe/releases); needs glibc 2.39 |
| Anything else | Build an archive on that machine with `scripts/tasks/standalone.sh` (needs rustup and Python 3.11) |

```bash
sha256sum -c bobe-daemon-<version>-<platform>.tar.gz.sha256
tar -xzf bobe-daemon-<version>-<platform>.tar.gz
./bobe-daemon-<version>-<platform>/install.sh
```

The installer puts BoBe in `~/.local/share/bobe/versions/<version>/` and links
`~/.local/bin/bobe` (`BOBE_INSTALL_PREFIX` changes `~/.local`). The archive's
`uninstall.sh` removes the service and files and keeps `~/.bobe/`. From a
checkout, `scripts/tasks/backend.sh` runs a development daemon.

## Set up

```bash
bobe setup
bobe chat
```

`bobe setup` installs the service, picks a provider and model, asks your name,
optionally opens network access and pairs a device. It skips finished steps.

The service is a launchd agent (`com.bobe.daemon`) on macOS and a systemd user
unit (`bobe.service`) on Linux. It keeps running, and spending on a cloud
provider, until `bobe service uninstall`. `bobe serve` runs the daemon in the
foreground on `127.0.0.1:8766`.

## Providers and models

```bash
bobe provider upsert openai --kind open-ai
printf '%s\n' "$OPENAI_API_KEY" | bobe provider credential openai
bobe model list --profile openai
bobe provider activate openai --kind open-ai \
  --chat-model <model> --goals-model <model> --consolidate-model <model>
bobe service restart
```

Keys are read from stdin so they stay out of shell history. Kinds: `open-ai`,
`anthropic`, `deepseek`, `open-router`, `rail-relay`, `copilot`,
`open-ai-compatible`. For a sign-in, create the profile with
`--auth-method oauth` and run `bobe provider auth login --profile <profile>`.
Anthropic is API-key-only.

Local models through Ollama:

```bash
bobe provider activate local --kind open-ai-compatible \
  --base-url http://127.0.0.1:11434/v1 --local-runtime ollama --offline \
  --chat-model qwen2.5:7b-instruct --goals-model qwen2.5:7b-instruct \
  --consolidate-model qwen2.5:7b-instruct
bobe local-runtime install --chat-model qwen2.5:7b-instruct --batch-model qwen2.5:7b-instruct
bobe service restart
```

`bobe local-runtime install` uses an Ollama that is already running or
installs BoBe's pinned Ollama under the data root, then pulls the models into
Ollama's own `~/.ollama/`. When the daemon stops, it stops the Ollama it
started and any model that Ollama loaded; a later start does not start it
again, so run Ollama as your own service if local models must survive a
restart. BoBe starts Ollama with a 40,960-token context so its prompts are not
cut off; start your own Ollama with `OLLAMA_CONTEXT_LENGTH=40960` as well.

| Workload | Used for |
|---|---|
| `chat` | Conversation |
| `goals` | Goal work and background jobs |
| `small` | Quick decisions, summaries, screen context |
| `consolidate` | Tidying memory |

`bobe model use <profile> <model> --workload <name|all>` reassigns one.
Provider, credential and model changes apply after `bobe service restart`.
`bobe doctor --probe` makes a small real request to test execution, streaming
and tools.

## Working with BoBe

| Command | Use |
|---|---|
| `bobe chat ["message"]` | Interactive, or one message. The CLI has its own thread |
| `bobe runs list \| stop \| guide <id> "text"` | Current work; `--follow-up` applies guidance after the current task |
| `bobe jobs list \| show \| cancel \| resume` | Background jobs. Interrupted jobs wait for `resume` |
| `bobe schedules create --at <time> [--every-seconds N] "title" "task"` | Planned work; also `list`, `pause`, `resume`, `reschedule`, `cancel` |
| `bobe input list \| show \| answer <id> "text" \| cancel` | Questions BoBe saved for you |
| `bobe agent status \| compact \| tools list \| tools disable <tool>` | Context, compaction and tool selection (restart to apply) |
| `bobe memory show \| replace --file <f>` | The memory document |
| `bobe profile list \| create \| disable` | User profiles |
| `bobe skill list \| show \| reload` | Skills; `reload` after editing skill files by hand |
| `bobe workspace path \| list \| read` | BoBe's working folder |
| `bobe voice-models status \| install [--kind tts] \| cancel` | Daemon speech models |
| `bobe voice preview "text" --output <wav> [--play]` | Speech without a provider ([speech.md](speech.md#preview-without-a-provider)) |

Disabling a tool is not a sandbox: tools run with your account's permissions
from `workspace/`.

## Configuration

```bash
bobe config show
bobe config set capture.enabled false
bobe config unset voice.speed
```

| Key | Meaning |
|---|---|
| `capture.enabled` | Screen awareness |
| `checkin.enabled`, `checkin.times` | Scheduled check-ins |
| `goals.enabled`, `memory.enabled` | Goals and memory, with their tools |
| `voice.enabled`, `voice.persona`, `voice.speed`, `voice.stt_language` | Voice |
| `mcp.enabled` | MCP tools |
| `server.host`, `server.port` | Where the daemon listens |

Environment variables override keys as `BOBE_<SECTION>__<KEY>`, for example
`BOBE_CAPTURE__ENABLED=false`. Providers and routes are managed only with
`bobe provider` and `bobe model`.

Data lives in `~/.bobe/` (`BOBE_DATA_DIR` moves it). One daemon may use a data
root at a time.

| Path | Holds |
|---|---|
| `config.toml` | Configuration |
| `data/` | SQLite: conversations, jobs, schedules, questions, devices |
| `memory.md`, `goals/` | Memory and goals |
| `skills/`, `workspace/` | Skills and BoBe's working folder |
| `agent/` | Transcripts and provider sign-ins |
| `headless-secrets.json` | API keys (mode 0600) |
| `models/`, `backups/`, `logs/` | Speech models, backups, logs |
| `ollama/` | The Ollama BoBe installed; its models stay in `~/.ollama/` |

## Remote access and pairing

```bash
bobe config remote --host 0.0.0.0 --allowed-host bobe.example.net
bobe service restart
bobe device pair                 # a Mac: six-digit code
bobe device pair --kind phone    # an iPhone: QR code with a bobe://pair link
bobe device list
bobe device revoke <id>
```

Codes work once and expire after five minutes. The daemon creates and renews
its own TLS identity (`--tls-cert`/`--tls-key` supply your own); network
traffic is always HTTPS. Opening a firewall or router is up to you. A phone
can pair only with a daemon reachable on the network. Physical bodies enroll
with `bobe endpoint` ([ESP32 SDK](../esp32-sdk/README.md)).

## Backup and restore

```bash
bobe backup create
bobe backup list
bobe backup verify <path>
bobe backup restore <path>
```

Backups exclude provider sign-ins. A backup restores into a BoBe with the
same backup format and the same or a newer schema; an older database is
upgraded at the next start.

## Update

The Mac app updates with **Check for Updates…**. A standalone daemon applies a
signed release:

```bash
bobe update apply \
  --manifest bobe-daemon-X.Y.Z-<platform>.tar.gz.manifest.json \
  --signature bobe-daemon-X.Y.Z-<platform>.tar.gz.manifest.json.sig \
  --archive bobe-daemon-X.Y.Z-<platform>.tar.gz
```

It verifies the signature and hash, checks that the new version can upgrade
the database, backs up, and rolls back if the new version does not start.

## Troubleshooting

| Problem | Fix |
|---|---|
| `bobe: command not found` | Add `~/.local/bin` to `PATH` |
| A checkout daemon cannot start the agent | Use `scripts/tasks/backend.sh`, or set `BOBE_NODE_PATH=build/agent-runtime/node` and `BOBE_AGENT_ENTRY=build/agent-runtime/agent/dist/main.js` |
| `database schema version N is newer than …` | The data was written by a newer BoBe; run that release |
| `database schema version N cannot be upgraded …` | The data is too old to upgrade; move `~/.bobe/` aside and set up again |
| A second daemon refuses to start | Stop the first, or give the second its own `BOBE_DATA_DIR` |
| Something else | `bobe status`, `bobe doctor`, `bobe service logs`, `~/.bobe/logs/` |
