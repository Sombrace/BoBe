# WB-12 hardware-in-the-loop tools

`tools/bobe_hil.sh` tests a real WB-12 running normal product firmware. Run it
from `examples/wb12`; `tools/bobe_hil.sh --help` lists every command, and
`tools/bobe_hil.sh doctor` reports which hardware and configuration are missing.
`private/` below is the ignored `examples/wb12/private/`.

```text
Mac Wi-Fi -> internet
Mac USB Ethernet -> OpenWrt One wired management
OpenWrt One 2.4 GHz radio -> WB-12 test and provisioning Wi-Fi
Mac USB -> WB-12 native USB Serial/JTAG
Mac USB -> Pico 2 fixture -> open-drain shunts on the WB-12 buttons and EN
Fixed camera under a hood -> WB-12 screen
Fixed speaker and reference microphone -> WB-12 microphone and speaker
```

## Evidence

Every result records its claim: `logical` (host render), `protocol` (BodyLink
test peer), `simulation` or `physical`. A weaker claim never counts as a
stronger one. Only a Raspberry Pi observer that signs its own measurements can
give physical audio evidence, and its audio loop returns `NOT_IMPLEMENTED`, so
no run claims physical audio.

[`scenarios/*.toml`](scenarios) declare steps, requirements and expected log
markers. `scenario run NAME --dry-run` records the plan without hardware,
`--mock` runs simulated adapters, and `--live` checks the declared hardware
first and ends `blocked` (exit 3) if any is missing. Cleanup always releases
every contact and network lease; a run that needed forced cleanup fails. Each
run writes `private/hil-runs/RUN_ID/` with a hash-chained `events.jsonl`, which
`evidence verify` rechecks. Hashes prove integrity, not what physically
happened.

`media record-audio` and `play-record` open a live microphone and keep what they
hear. With more than one WB-12 on USB, pass `--port`.

## Lab setup

| Part | Setup |
|---|---|
| Pico 2 fixture | `tools/build_pico_fixture.sh`, then copy `build-pico-fixture/bobe_fixture.uf2` to the Pico. Create `private/hil/fixture.json` from [the example](fixture/pico2/fixture-config.example.json) with `board_id` set to the Pico's lowercase USB serial number |
| OpenWrt One | Run the release in [`openwrt-one.lock`](router/openwrt-one.lock). Install [`bobe-netem`](router/bobe-netem) as `/usr/sbin/bobe-netem` and [its config](router/bobe-netem.conf.example) as root-owned `/etc/bobe-hil/netem.conf` (mode 0600); the test interface must never be the management interface |
| Mac network | USB Ethernet to the router's LAN port with a static address and no gateway or DNS. `private/hil/network/` holds `openwrt.json` ([example](router/network-config.example.json)), `known_hosts` pinned to the router, and `router_ed25519` (mode 0600) |
| Audio observer (Pi) | [`bobe-observer.service`](observer/systemd/bobe-observer.service) runs [`bobe_observer`](../bobe_observer) unprivileged with `/etc/bobe-observer/observer.json` ([example](observer/service-config.example.json)); `python -I -m bobe_observer check-config` validates it |
| Audio observer (Mac) | `private/hil/observer/` holds `observer.json` ([example](observer/observer-config.example.json)), `observer-ca.pem`, `observer-server-cert.pem`, `mac-client-cert.pem` and `mac-client-key.pem` (mode 0600) |
| Camera | `tools/bobe_capture.sh` builds, signs and runs the macOS helper that holds the camera permission; always use this wrapper |
| Logic analyzer | `tools/bootstrap_sigrok_cli.sh`, then `tools/sigrok.sh`. A 24 MS/s FX2 analyzer cannot capture the 40 MHz display bus. Read the [sigrok skill](../.claude/skills/sigrok-cli/SKILL.md) before connecting an unverified analyzer |
| Serial log | `uv run --locked python tools/capture_serial.py PORT private/device-logs/run.log --duration 60` writes a timestamped log that follows USB resets |

Every `net apply` impairment has a TTL of 5–600 s; the router clears it when the
TTL ends, even if the Mac process dies.

## Calibration

Copy [`camera.example.json`](calibration/camera.example.json) and
[`audio.example.json`](calibration/audio.example.json) to
`private/hil/calibration/camera.json` and `audio.json` and fill in measured
values; the zero thresholds are placeholders. A record binds the exact devices
and settings it was measured with; any change of camera, lighting, mounting,
gain or geometry invalidates it. `doctor` flags an expired record; distrust
results that use one.

## Fixture wiring

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

The [fixture firmware](fixture/pico2/main.c) accepts only bounded commands and
releases every contact after its 1 s watchdog or 30 s session timeout.
