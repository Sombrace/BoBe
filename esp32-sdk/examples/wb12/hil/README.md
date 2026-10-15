# WB-12 hardware-in-the-loop tools

`tools/bobe_hil.sh` tests a real WB-12 running normal product firmware. There is
no test image and no remote-control shell on the device: the tool presses buttons
through a Pico 2 fixture, reads the production serial log, impairs Wi-Fi through
an OpenWrt router, and captures the screen and sound. It runs unprivileged from
the project environment (`uv run --group hil --locked`) and installs nothing
globally. Run commands from `examples/wb12`; `private/` below is
`examples/wb12/private/`, which Git ignores.

Every result records one claim: `logical` (a host render), `protocol` (the
BodyLink test peer), `simulation` (`--mock`) or `physical` (real hardware). A
weaker claim never counts as a stronger one.

```text
Mac Wi-Fi -> internet
Mac USB Ethernet -> OpenWrt One wired management
OpenWrt One 2.4 GHz radio -> WB-12 test and provisioning Wi-Fi
Mac USB -> WB-12 native USB Serial/JTAG
Mac USB -> Pico 2 fixture -> open-drain shunts on the WB-12 buttons and EN
Fixed camera under a hood -> WB-12 screen
Fixed speaker and reference microphone -> WB-12 microphone and speaker
```

## Commands

| Command | Does |
|---|---|
| `doctor` | Reports which hardware and configuration are present |
| `device status`, `device identify` | Lists WB-12s on USB; reads the MAC |
| `device observe --seconds 40 --expect "health uptime=" --reject "Guru Meditation"` | Watches the serial log for markers |
| `device flash --image IMAGE --rollback-image IMAGE` | Writes and verifies an application ([WB-12 README](../README.md#flash)) |
| `fixture status`, `fixture release-all` | Checks the Pico fixture; releases every contact |
| `fixture press --contact action --ms 250` | Presses `action`, `volume-down` or `volume-up` for 30–2000 ms |
| `fixture chord --name shutdown --ms 2000` | Holds the `shutdown` or `provision-recovery` chord |
| `fixture reset --ms 120`, `fixture enter-loader` | Pulses EN; enters the ROM loader |
| `net apply --profile blackhole --ttl 120`, `net status`, `net clear` | Router impairment leases |
| `observer status`, `observer probe` | Checks the audio observer's identity; one status request |
| `scenario list`, `scenario show NAME` | Lists scenarios and their requirements |
| `scenario run NAME --dry-run`, `--mock` or `--live`, optional `--junit FILE` | Runs a scenario |
| `bodylink serve handshake` or `voice-roundtrip` | Serves one BodyLink session on port 8767 with `private/bodylink-pki/` |
| `ui render` | Renders every face with the production renderer and compares pixels |
| `media ...` | Lists audio devices, records, plays and compares audio and images |
| `artifact create-manifest BUILD_DIR --profile PROFILE` | Writes an unsigned firmware manifest |
| `evidence verify private/hil-runs/RUN_ID` | Rechecks a run's evidence |

Each command reports what it did, what it saw and which device it touched, or
names the missing hardware or configuration. With more than one WB-12 on USB,
pass `--port`. The tool finds the board again by its MAC after a USB reset and
fails if a different board answers. `media record-audio` and `play-record` open
a live microphone and keep what they hear.

## Scenarios

[`scenarios/*.toml`](scenarios) define each scenario's steps, durations,
requirements and expected or rejected log markers. `--dry-run` records the plan
without touching hardware. `--mock` runs simulated adapters and can never pass a
physical requirement. `--live` checks the declared hardware first and ends
`blocked` (exit code 3, skipped in JUnit) if any is missing. Serial capture
starts before the first press or reset and follows USB resets. Cleanup always
releases every contact and network lease, and a run that needed forced cleanup
fails.

Only one process uses a device at a time (locks in `private/hil/`). Each run
writes `private/hil-runs/RUN_ID/`: a manifest, a hash-chained `events.jsonl` and
artifacts such as serial logs, WAV, PNG and RGB565 files. `evidence verify`
rechecks the chain, the artifact hashes and, for a physical pass, the step
evidence and cleanup. Hashes prove integrity, not what physically happened.

## Pico 2 button fixture

```sh
tools/build_pico_fixture.sh
```

This fetches the pinned Pico SDK and RISC-V toolchain
([`fixture/pico2/toolchain.lock`](fixture/pico2/toolchain.lock)) into `.deps/` and
builds `build-pico-fixture/bobe_fixture.uf2`. Copying the UF2 to the Pico is a
manual step. Copy [`fixture-config.example.json`](fixture/pico2/fixture-config.example.json)
to `private/hil/fixture.json` and set `board_id` to the Pico's USB serial number
in lowercase.

| Pico pin | WB-12 contact |
|---|---|
| GP2 | GPIO0 switch pad (main button) |
| GP3 | GPIO39 switch pad (volume down) |
| GP4 | GPIO40 switch pad (volume up) |
| GP5 | EN switch pad (reset) |
| GP6 | Enable input of the GPIO0 loader gate |

> [!WARNING]
> Never connect a Pico pin straight to the toy. Drive each contact through its
> own open-drain stage: a 2N7002 or BSS138 with the drain on the switch signal
> and the source on verified toy ground, a 100 kΩ gate pull-down and a
> 100–1,000 Ω gate resistor. Join the grounds only after measuring voltages and
> contacts, and prefer normally-open PhotoMOS relays until a meter confirms the
> pads and their grounded sides.
>
> GPIO0 also selects the ROM loader at reset, so it needs a hardware gate: the
> action shunt may close only while EN is high, unless GP6 and the action drive
> are both asserted. GP6 needs its own pull-down. Unplugging USB does not
> power-cycle the battery-powered WB-12; do not add a battery switch until its
> power path has been measured.

The fixture firmware ([`fixture/pico2/main.c`](fixture/pico2/main.c)) accepts only
bounded press, chord, reset, loader and release commands. A 1 s watchdog, a 30 s
session timeout and the gate pull-downs release every contact. Reset and loader
timings are fixed in the firmware.

## OpenWrt test network

The router is an OpenWrt One running the release pinned in
[`router/openwrt-one.lock`](router/openwrt-one.lock). Its 1 GbE LAN port goes to a
Mac USB Ethernet adapter with a static address and no gateway or DNS, so the
Mac's internet stays on its own Wi-Fi. Its 2.4 GHz radio is the WB-12's network.

- On the router, install [`router/bobe-netem`](router/bobe-netem) as
  `/usr/sbin/bobe-netem` and [`router/bobe-netem.conf.example`](router/bobe-netem.conf.example)
  as root-owned `/etc/bobe-hil/netem.conf` (mode 0600). The test interface must
  never be the wired management interface.
- On the Mac, put in `private/hil/network/`: `openwrt.json` from
  [`router/network-config.example.json`](router/network-config.example.json) with
  the router's fixed management IPv4 address, `known_hosts` pinned to that
  address and the router's host key, and `router_ed25519` (mode 0600).

`net apply` takes `clean`, `delay-150`, `loss-1`, `loss-5`, `jitter-loss`,
`rate-512` or `blackhole`, with a TTL of 5–600 s. The router removes the
impairment when the TTL ends, even if the Mac process dies. `bobe-netem` is the
only command run over SSH. Every profile except `blackhole` shapes only traffic
toward the WB-12.

## Audio observer

Audio recorded on the Mac does not count as physical evidence, because anything
in the Mac session could alter it. Physical audio evidence must come from a
Raspberry Pi observer that signs its own measurements. Its service
([`bobe_observer`](../bobe_observer)) answers status and artifact requests and
returns `NOT_IMPLEMENTED` for the audio loop, so no run can claim physical audio.

- On the Pi, [`observer/systemd/bobe-observer.service`](observer/systemd/bobe-observer.service)
  runs `/opt/bobe-observer/venv/bin/python -I -m bobe_observer serve` as an
  unprivileged user with no device access. Its configuration is
  `/etc/bobe-observer/observer.json` (from
  [`observer/service-config.example.json`](observer/service-config.example.json)),
  which `python -I -m bobe_observer check-config` validates.
- On the Mac, `private/hil/observer/` holds `observer.json` (from
  [`observer/observer-config.example.json`](observer/observer-config.example.json)),
  `observer-ca.pem`, `observer-server-cert.pem`, `mac-client-cert.pem` and
  `mac-client-key.pem` (mode 0600).

`observer status` checks this configuration without connecting; `observer probe`
makes one pinned mutual-TLS status request.

## Calibration

Copy [`calibration/camera.example.json`](calibration/camera.example.json) and
[`calibration/audio.example.json`](calibration/audio.example.json) to
`private/hil/calibration/camera.json` and `audio.json`, and fill in measured
values; the zero thresholds are placeholders. A camera record binds the exact
camera ID, output size, orientation, mirroring and region of interest. An audio
record binds the exact input and output device names, sample rate and pass
limits. `doctor` reports an expired record without blocking; distrust results
that use it. A change of camera, lighting, mounting, gain or geometry
invalidates the record.

## Camera, logic analyzer and serial log

- `tools/bobe_capture.sh` builds and signs the macOS camera helper, which owns
  the camera permission, so always call the helper through this wrapper. Its
  commands are `list`, `preflight`, `authorize`, `frame`, `rectify` and `ocr`.
- `tools/bootstrap_sigrok_cli.sh` builds the pinned native `sigrok-cli`
  ([`tools/sigrok-cli.lock`](../tools/sigrok-cli.lock)) into `.deps/` on Apple
  silicon; `tools/sigrok.sh` runs bounded discovery, probes and captures. A
  24 MS/s FX2 analyzer cannot capture the 40 MHz display bus. Read the
  [sigrok skill](../.claude/skills/sigrok-cli/SKILL.md) before connecting an
  unverified analyzer.
- `uv run --locked python tools/capture_serial.py PORT private/device-logs/run.log --duration 60`
  writes a timestamped log that continues across USB resets. Without
  `--duration` it runs until Ctrl-C, and it refuses an existing log unless given
  `--append`.

Tool versions are pinned in `dependencies.lock`, `uv.lock` (the `hil` group),
`hil/fixture/pico2/toolchain.lock`, `hil/router/openwrt-one.lock` and
`tools/sigrok-cli.lock`. `tools/bobe_hil.sh --version` and
`tools/bobe_capture.sh --version` report the lab tools.
