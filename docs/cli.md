# Command line

`bobe` is both the daemon and its control client. Run any command with
`--help`; most accept `--json`, and `--url` and `--token` address another BoBe.

## Install

| Machine | How |
|---|---|
| Mac with the app | **Settings → Runtime → Install Command Line Tool…** links `~/.local/bin/bobe` to the app's copy |
| Linux x86_64 | Download `bobe-daemon-<version>-linux-x86_64.tar.gz` and its `.sha256` from [Releases](https://github.com/Sombrace/BoBe/releases); needs glibc 2.39 and `ca-certificates` |
| Anything else | Build an archive there with `scripts/tasks/standalone.sh` (rustup, Python 3.11 and, on Linux, `dpkg-deb`) |

```bash
sha256sum -c bobe-daemon-<version>-<platform>.tar.gz.sha256
tar -xzf bobe-daemon-<version>-<platform>.tar.gz
./bobe-daemon-<version>-<platform>/install.sh
```

The installer puts BoBe in `~/.local/share/bobe/versions/<version>/` and links
`~/.local/bin/bobe` (`BOBE_INSTALL_PREFIX` replaces `~/.local`). The archive's
`uninstall.sh` removes the service and files and keeps `~/.bobe/`.

## Set up

```bash
bobe setup
bobe chat
```

`bobe setup` installs the service, picks a provider and model, asks your name,
optionally opens network access and pairs a device; it skips finished steps.
The service is a launchd agent (`com.bobe.daemon`) on macOS and a systemd user
unit (`bobe.service`) on Linux, and runs (and spends on a cloud provider)
until `bobe service uninstall`. `bobe serve` runs the daemon in the foreground
on `127.0.0.1:8766`.

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

`bobe local-runtime install` uses a running Ollama or installs BoBe's pinned
one under the data root; models go to Ollama's own `~/.ollama/`. BoBe starts
its Ollama with the daemon whenever a route uses it, and stops it with the
daemon. If you run Ollama yourself, start it with `OLLAMA_CONTEXT_LENGTH=40960`
(BoBe's prompts need that context).

| Workload | Used for |
|---|---|
| `chat` | Conversation |
| `goals` | Goal work and background jobs |
| `small` | Quick decisions, summaries, screen context |
| `consolidate` | Tidying memory |

`bobe model use <profile> <model> --workload <name|all>` reassigns one.
Provider, credential and model changes apply after `bobe service restart`, and
`bobe doctor --probe` makes a small real request to test them.

## Working with BoBe

| Command | Use |
|---|---|
| `bobe chat ["message"]` | Interactive, or one message, in the CLI's own thread |
| `bobe runs list \| stop \| guide <id> "text"` | Current work; `--follow-up` guides after the current task |
| `bobe jobs list \| show \| cancel \| resume` | Background jobs; interrupted jobs wait for `resume` |
| `bobe schedules create --at <time> [--every-seconds N] "title" "task"` | Planned work; also `list`, `pause`, `resume`, `reschedule`, `cancel` |
| `bobe input list \| show \| answer <id> "text" \| cancel` | Questions BoBe saved for you |
| `bobe agent status \| compact \| tools list \| tools disable <tool>` | Context, compaction and tools (restart to apply) |
| `bobe memory show \| replace --file <f>` | The memory document |
| `bobe profile list \| create \| disable` | User profiles |
| `bobe skill list \| show \| reload` | Skills; `reload` after editing skill files by hand |
| `bobe workspace path \| list \| read` | BoBe's working folder |
| `bobe voice-models status \| install [--kind tts] \| cancel` | Daemon speech models |
| `bobe voice preview "text" --output <wav> [--play]` | Speech without a provider |

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

`BOBE_<SECTION>__<KEY>` environment variables override keys, for example
`BOBE_CAPTURE__ENABLED=false`. Providers and routes change only through
`bobe provider` and `bobe model`.

Data lives in `~/.bobe/` (`BOBE_DATA_DIR` moves it); one daemon uses a data
root at a time.

| Path | Holds |
|---|---|
| `config.toml` | Configuration |
| `data/` | SQLite: conversations, jobs, schedules, questions, devices |
| `memory.md`, `goals/` | Memory and goals |
| `skills/`, `workspace/` | Skills and BoBe's working folder |
| `agent/` | Transcripts and provider sign-ins |
| `headless-secrets.json` | API keys (mode 0600) |
| `models/`, `ollama/` | Speech models and the Ollama BoBe installed |
| `backups/`, `logs/` | Backups and logs |

## Remote access and pairing

```bash
bobe config remote --host 0.0.0.0 --allowed-host bobe.example.net
bobe service restart
bobe device pair                 # a Mac: six-digit code
bobe device pair --kind phone    # an iPhone: QR code with a bobe://pair link
bobe device list
bobe device revoke <id>
```

Codes work once and expire after five minutes. Network traffic is always
HTTPS with a certificate the daemon creates and renews (`--tls-cert` and
`--tls-key` supply your own); opening a firewall or router is up to you.
Physical bodies enroll with `bobe endpoint` ([ESP32 SDK](../esp32-sdk/README.md)).

## Backup, restore and update

```bash
bobe backup create
bobe backup list
bobe backup verify <path>
bobe backup restore <path>
bobe update apply --manifest <archive>.manifest.json \
  --signature <archive>.manifest.json.sig --archive <archive>
```

Backups exclude provider sign-ins and restore into the same or a newer
schema; a restore interrupted by a crash is finished or undone at the next
start. `bobe update apply` installs a signed standalone release: it checks the
signature, hash and database compatibility, backs up, and rolls back if the
new version never becomes ready, saving the current data as a new backup
first. An update interrupted after the new version became ready is finished,
not rolled back. The Mac app updates with **Check for Updates…**.

## Troubleshooting

| Problem | Fix |
|---|---|
| `bobe: command not found` | Add `~/.local/bin` to `PATH` |
| A checkout daemon cannot start the agent | Use `scripts/tasks/backend.sh`, or set `BOBE_NODE_PATH=build/agent-runtime/node` and `BOBE_AGENT_ENTRY=build/agent-runtime/agent/dist/main.js` |
| `database schema version N is newer than …` | A newer BoBe wrote the data; run that release |
| `database schema version N cannot be upgraded …` | The data is too old; move `~/.bobe/` aside and set up again |
| A second daemon refuses to start | Stop the first, or give the second its own `BOBE_DATA_DIR` |
| Something else | `bobe status`, `bobe doctor`, `bobe service logs`, `~/.bobe/logs/` |
