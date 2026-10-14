# BoBe hardware-in-the-loop validation

This directory holds the scenarios and appliance sources used by
`tools/bobe_hil.sh`. The device under test always runs normal product firmware;
there is no diagnostic image or on-device control plane, and every observation
comes from outside. Evidence has three layers, and no lower layer can claim a
higher one:

| Layer | Evidence |
|---|---|
| Native host | Core policy, protocol, media metrics and canonical LVGL RGB565 |
| Protocol | Real mTLS WebSocket BodyLink framing, routing, credit and terminal reports |
| Physical fixture | Electrical buttons and EN, production serial logs, optical LCD, acoustic audio and isolated Wi-Fi faults |

## Commands

`tools/bobe_hil.sh` runs unprivileged through `uv run --group hil --locked`, with
no daemon, socket, launchd job or system installation.

```bash
tools/bobe_hil.sh doctor
tools/bobe_hil.sh device status
tools/bobe_hil.sh device identify
tools/bobe_hil.sh device observe --seconds 40 --expect "health uptime=" \
    --reject "Guru Meditation"
tools/bobe_hil.sh device flash --image build-bobe-snapshot/bobe_wb12.bin \
    --offset 0x20000 --rollback-image build-ondevice-reviewed/bobe_wb12.bin
tools/bobe_hil.sh fixture press --contact action --ms 250
tools/bobe_hil.sh fixture chord --name shutdown --ms 2000
tools/bobe_hil.sh fixture reset --ms 120
tools/bobe_hil.sh fixture enter-loader
tools/bobe_hil.sh fixture release-all
tools/bobe_hil.sh net apply --profile blackhole --ttl 120
tools/bobe_hil.sh net status
tools/bobe_hil.sh net clear
tools/bobe_hil.sh observer status
tools/bobe_hil.sh observer probe
tools/bobe_hil.sh scenario list
tools/bobe_hil.sh scenario show physical-controls
tools/bobe_hil.sh scenario run physical-controls --dry-run
tools/bobe_hil.sh scenario run voice-roundtrip --mock --junit .tmp/mock.xml
tools/bobe_hil.sh scenario run offline-boot --live --junit .tmp/offline.xml
tools/bobe_hil.sh ui render
tools/bobe_hil.sh evidence verify private/hil-runs/RUN_ID
```

Every physical command reports what it did, what it observed and which device it
touched; a command that cannot run names the missing hardware or configuration.
The tool uses the one WB-12 connected over USB; with several, pass `--port`, and
live scenarios that need the WB-12 report `blocked`. An operation records the
board's MAC from its USB serial number, uses it to find the board after a USB
reset, and fails if a different board answers.

## Scenarios

`hil/scenarios/*.toml` are the executable source of truth: durations, contacts,
profiles, TTLs and expected or rejected markers come from the file.

| Operation | Implementation |
|---|---|
| `fixture_press`, `fixture_chord`, `fixture_reset`, `fixture_release_all` | `bobe_hil/fixture.py` |
| `network_profile` | `bobe_hil/network.py` (`clean` clears the active lease) |
| `capture_serial` | `bobe_hil/device.py`, reading production logs |
| `capture_audio` | `bobe_hil/media.py` with the `hil/calibration` device names |
| `capture_frame` | `tools/bobe_capture.sh frame` with the calibrated camera ID |
| `bodylink_stub`, `bodylink_stop` | `bobe_hil/bodylink.py` |
| `ui_golden` | `bobe_hil/ui.py` |

`--dry-run` records the plan without touching hardware (status `planned`, exit
0). `--mock` runs deterministic synthetic adapters under a `simulation` claim that
never satisfies a physical requirement. `--live` runs the real steps after a
hardware preflight and exits 3 with `blocked` when a declared requirement is
absent.

A live serial monitor opens before the first actuating step, so output during
presses and resets is captured, and it reconnects across USB re-enumeration.
Cleanup always runs and releases every contact and network lease used; a run that
needed forced cleanup fails.

## Physical invariants

- One process per device (`fcntl.flock` on `private/hil/*.lock`).
- ROM loader sequencing chains `--before/--after no-reset` and waits, bounded, for
  USB re-enumeration after fixture-assisted loader entry.
- Writes are read back and verified. When the target is the selected OTA slot and
  a manifest names the ELF, the observed boot is bound to the written image
  through the bootloader ELF digest and the production identity line.
- The tool never writes to the device serial port; it only reads normal logs.
- Fixture contacts are released on every exit path.
- Router impairment leases expire on their own TTL (5–600 s), so the test LAN
  heals if the process dies.
- Secure Boot and flash-encryption state is reported as a warning before a write,
  not a refusal.

## Private configuration

Hardware identities live in the ignored `private/hil/`:

```text
private/hil/fixture.json              # Pico board_id; see hil/fixture/pico2/fixture-config.example.json
private/hil/network/openwrt.json      # router IP; see hil/router/network-config.example.json
private/hil/network/router_ed25519    # SSH key, mode 0600
private/hil/network/known_hosts       # pinned router host key
private/hil/observer/                 # observer identity and pinned TLS material
private/hil/calibration/camera.json   # ROI, geometry, comparison thresholds
private/hil/calibration/audio.json    # device names, rate, thresholds
```

Calibration is reference data; expiry is reported and never blocks a tool.

## Toolchain

ESP-IDF 6.0.2, esptool 5.5.0 and LVGL 9.5.0; standard-library `unittest` host
tests with no auto-flash plugin; Pillow 12.3.0, NumPy 2.4.2, sounddevice 0.5.5 and
websockets 16.1.1 in the `hil` uv group; Apple AVFoundation, Vision and CoreImage
through the ad-hoc-signed `bobe-capture` helper; Pico SDK 2.3.0 for the fixture.
Toolchains live under the ignored `.deps/`; nothing is installed globally or with
`sudo`. [HIL toolchain](../docs/hil-toolchain.md) records the choices.

## Physical topology

```text
Mac internet Wi-Fi
Mac USB Ethernet -> OpenWrt One wired management
OpenWrt One 2.4 GHz radio -> WB-12 test and provisioning Wi-Fi
Mac USB -> WB-12 native USB Serial/JTAG
Mac USB -> Pico 2 fixture -> open-drain button and EN shunts
Fixed UVC camera and hood -> WB-12 LCD
Fixed audio stimulus and reference capture -> WB-12 microphone and speaker
```

The Mac Ethernet interface has a static address with no gateway or DNS; internet
routing stays on Wi-Fi.

## Evidence

Each run writes `private/hil-runs/<run>/` with a manifest, a hash-chained
`events.jsonl` and artifacts (serial logs, WAV, PNG, RGB565). `bobe-hil evidence
verify` rechecks the chain, the artifact digests, and that a physical pass has
completed step evidence and clean cleanup. Hashes prove integrity, not physical
causality. Application manifests may carry an Ed25519 signature checked against
`hil/security/manifest-signing-public.pem`; `bobe-hil artifact create-manifest`
writes an unsigned record, and `device flash` binds an image to its manifest
digests when one is present.

`doctor` and each command report which parts of the topology are present.
Battery cold-cycle and shutdown recovery also need a characterized charger, load
path and battery harness. Scenarios that need an absent fixture are `blocked`,
never simulated passes.
