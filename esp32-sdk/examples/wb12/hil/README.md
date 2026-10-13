# BoBe autonomous validation

This directory contains the declared scenarios and appliance-side sources used
by `tools/bobe_hil.sh`. The device under test always runs normal BoBe product
firmware; there is no instrumented diagnostic image and no on-device control
plane. Every observation comes from outside the device. It separates three
evidence layers:

| Layer | Evidence |
|---|---|
| Native host | Pure core policy, protocol, media metrics, and canonical LVGL RGB565 |
| Protocol | Real mTLS/WebSocket BodyLink framing, routing, credit, and terminal reports |
| Physical fixture | Electrical buttons/EN, production serial logs, optical LCD, acoustic audio, and isolated Wi-Fi faults |

No lower layer can claim success for a higher layer.

## Commands

`tools/bobe_hil.sh` is the project-local hardware tool. It runs unprivileged in
the repository through `uv run --group hil --locked`; there is no daemon,
socket, launchd job, or system installation.

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

Every physical command reports what it did, what it observed, and which device
it touched. Commands that cannot run report the missing hardware or missing
configuration as a fact; nothing is withheld by permission flags.

`tools/bobe_hil.sh` uses the one WB-12 that is connected over USB. When several
are connected, pass `--port` to a `device` command. A live scenario that needs
the WB-12 reports `blocked` while several are connected. An operation takes the
board's MAC from its USB serial number when it starts. It uses that MAC to find
the board after a USB reset, and it fails if a different board answers.

## Scenarios

`hil/scenarios/*.toml` are the executable source of truth. The runner parses a
scenario and dispatches each step directly to the local implementation:

| Operation | Implementation |
|---|---|
| `fixture_press`, `fixture_chord`, `fixture_reset`, `fixture_release_all` | `bobe_hil/fixture.py` |
| `network_profile` | `bobe_hil/network.py` (`clean` clears the active lease) |
| `capture_serial` | `bobe_hil/device.py` serial monitor over production logs |
| `capture_audio` | `bobe_hil/media.py` with `hil/calibration` device names |
| `capture_frame` | `tools/bobe_capture.sh frame` with the calibrated camera ID |
| `bodylink_stub`, `bodylink_stop` | `bobe_hil/bodylink.py` |
| `ui_golden` | `bobe_hil/ui.py` |

Editing a TOML changes what runs. Step durations, contacts, profiles, TTLs and
expected/rejected markers are read from the file, not from a Python copy.

`--dry-run` prints and records the plan without touching hardware; its status is
`planned` and its exit code is 0. `--mock` executes deterministic synthetic
adapters and records the `simulation` claim with the intended `target_claim`;
mock evidence never satisfies a physical requirement. `--live` executes the
real steps after a hardware availability preflight and blocks (exit 3) when a
declared requirement is not physically present.

A live serial monitor opens before the first actuating step, so device output
emitted during button presses or a reset is captured rather than lost. The
monitor reconnects across USB re-enumeration and records disconnect counts.

Cleanup always runs, including after a failed step. The backend additionally
releases every fixture contact it used and clears any network lease it applied.
A run that needed forced cleanup fails; it never reports a physical pass.

## Physical invariants kept as engineering

- One process at a time per device (`fcntl.flock` on `private/hil/*.lock`).
- ROM loader sequencing with `--before/--after no-reset` chaining, and a bounded
  wait for USB re-enumeration after fixture-assisted loader entry.
- Write, then read back and verify. When the target is the selected OTA slot and
  a manifest identifies the ELF, bind the observed boot to the written image
  through the bootloader ELF digest and the production identity log line;
  otherwise report the boot observation without that attribution.
- The tool never writes commands to the device serial port. It only reads what
  normal firmware logs.
- Fixture contacts are released on every exit path.
- Router-side impairment leases expire on their own TTL (5-600 s), so the test
  LAN self-heals if this process dies.
- Secure boot and flash encryption state is read and reported as a warning
  before a write; it is evidence, not a refusal.

## Private configuration

Hardware identities live in the gitignored repository-local `private/hil/`:

```text
private/hil/fixture.json              # Pico board_id; see hil/fixture/pico2/fixture-config.example.json
private/hil/network/openwrt.json      # router IP; see hil/router/network-config.example.json
private/hil/network/router_ed25519    # SSH key, mode 0600
private/hil/network/known_hosts       # pinned router host key
private/hil/observer/                 # observer identity and pinned TLS material
private/hil/calibration/camera.json   # ROI, geometry, comparison thresholds
private/hil/calibration/audio.json    # exact device names, rate, thresholds
```

Calibration is reference data. Expiry is reported; it never blocks a tool.

## Toolchain

- ESP-IDF 6.0.2, esptool 5.5.0, LVGL 9.5.0.
- Python host tests use the standard-library `unittest` runner; no ESP auto-flash
  test plugin is installed.
- Pillow 12.3.0 and NumPy 2.4.2 for deterministic metrics.
- sounddevice 0.5.5 for project-local CoreAudio access.
- websockets 16.1.1 for the BodyLink stub.
- Apple AVFoundation/Vision/CoreImage through the ad-hoc-codesigned
  `bobe-capture` helper.
- Raspberry Pi Pico SDK/tool bundles 2.3.0 for fixture firmware.

All Python packages are pinned in the `hil` uv dependency group. Pico and IDF
toolchains live under ignored `.deps/`; nothing is installed globally or with
`sudo`.

## Required physical topology

```text
Mac internet Wi-Fi
Mac USB Ethernet -> OpenWrt One wired management
OpenWrt One 2.4 GHz radio -> WB-12 test/provisioning Wi-Fi
Mac USB -> WB-12 native USB Serial/JTAG
Mac USB -> Pico 2 fixture -> open-drain button/EN shunts
Fixed UVC camera/hood -> WB-12 LCD
Fixed audio stimulus/reference capture -> WB-12 mic/speaker
```

The Mac Ethernet interface must have a static address and no default gateway or
DNS. Internet routing remains on the existing Wi-Fi interface.

## Evidence

Every run writes a directory under `private/hil-runs/` holding a manifest, a
hash-chained `events.jsonl`, and retained artifacts (serial logs, WAV, PNG,
RGB565). `bobe-hil evidence verify` rechecks the chain, the artifact digests,
and that a physical pass contains completed step evidence and a clean cleanup
finalization. Hashes establish integrity, not physical causality: only the
device, fixture and observer records say what physically happened.

Application manifests can carry an Ed25519 signature verified against
`hil/security/manifest-signing-public.pem`. `bobe-hil artifact create-manifest`
produces an unsigned record; `bobe-hil device flash` binds an image to its
manifest digests when a manifest is present.

## Fixture availability

`doctor` and each command report which parts of the topology are present,
including the independent Pi observer. Battery cold-cycle and shutdown recovery
also need a characterized charger, load path and battery harness. Live
scenarios that need an absent fixture are `blocked`, never simulated passes.
