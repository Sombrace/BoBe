# Developer tools

Every tool runs unprivileged inside the repository through `uv`; nothing installs
globally or needs `sudo`.

| Tool | Purpose |
|---|---|
| `bootstrap_idf.sh` | Prepare ESP-IDF in the SDK checkout (default 6.0.2) |
| `idf.sh` | Run the project-local ESP-IDF environment |
| `check.sh` | Host tests and compile-only firmware builds |
| `build_bobe_snapshot.sh` | Build normal firmware from the committed Git snapshot with provenance hashes |
| `bobe_hil.sh` | External lab tool for the device, Pico fixture, router impairment, observer, scenarios and UI and media evidence ([HIL](../hil/README.md)) |
| `bobe_capture.sh` | Build, sign and run the camera CLI; always use this TCC-owning wrapper |
| `flash_app_verified.sh` | Manifest-gated `ota_0` flash with read-back verification |
| `bobe_image.py` | Validate ESP32-S3 app descriptors, manifests and OTA selection |
| `capture_serial.py` | Write timestamped serial output to a durable log |
| `bootstrap_sigrok_cli.sh`, `sigrok.sh` | Build a hash-pinned native `sigrok-cli`; discover FX2 analyzers and run bounded captures |
| `bootstrap_pico_fixture.sh`, `build_pico_fixture.sh` | Fetch the pinned Pico 2 SDK and build the fixture firmware |
| `generate_provisioning_credentials.sh` | Create or explicitly rotate a private Security 2 credential bundle |
| `generate_bodylink_pki.sh` | Create a development BodyLink CA, server and device certificates and claim |
| `provision_bobe.sh` | Validate a matching private claim and provision through the Security 2 SoftAP |

```bash
tools/bootstrap_idf.sh
tools/check.sh
```

`check.sh` starts from fresh build configuration and runs host tests, sanitizers,
logical UI references, the Pico fixture firmware, macOS capture tooling, and the
fallback, Security 2 push-to-talk and Security 2 plus ESP-SR edge firmware
profiles. Every profile is compile-only and must keep the production button
event log that scenarios observe. Narrow gates are `core-policy`, `ui`, `fixture`
and `capture`, for example `tools/check.sh ui`.

## Snapshot builds and flashing

The snapshot build archives only the committed `esp32-sdk` subtree and builds a
product profile (`fallback`, `ptt` or `edge`, default `edge`). It writes
`build-bobe-snapshot/bobe-snapshot.lock` with the source commit, image size and
hash and both ELF hashes; the lock and application manifest travel with the build.

```bash
tools/build_bobe_snapshot.sh --profile edge
tools/bobe_hil.sh device flash \
    --image build-bobe-snapshot/bobe_wb12.bin \
    --rollback-image build-ondevice-reviewed/bobe_wb12.bin
```

`device flash` reports whether the manifest and selected OTA slot bound the
observed boot to the written image. `flash_app_verified.sh` is the fixed-path
equivalent for a serial port you name: it enters the ROM loader through the native
USB reset sequence, prints the MAC, verifies the `ota_0` selection, security state,
app descriptor and signed clean-source manifest, writes only `0x20000`, and
verifies the bytes before restarting. The EN fixture is the recovery path when the
native reset does not respond.

## Credentials and Wi-Fi

```bash
tools/generate_bodylink_pki.sh private/bodylink-pki <device-uuid>
tools/generate_provisioning_credentials.sh --create --device-id <device-uuid>
tools/generate_provisioning_credentials.sh --rotate --device-id <device-uuid>
```

Credentials never rotate implicitly. Rotation archives the previous C and private
manifest pair before publishing the replacement. Live provisioning writes a
private receipt after device preflight, claim commit, Wi-Fi configuration and
Wi-Fi apply, so a late failure can resume.

A development image can carry test Wi-Fi from the ignored `private/dev-wifi.env`:

```bash
BOBE_WIFI_SSID='<network>'
BOBE_WIFI_PASSWORD='<password>'
tools/build_dev_wifi.sh
```

Its ignored `sdkconfig`, ELF and image contain the credential. Flash it only long
enough to confirm the NVS-backed connection, then overwrite the application slot
with a product image without erasing NVS. Product firmware can instead change
Wi-Fi over USB-C without reflashing:

```bash
uv run --group hil --locked python tools/configure_bobe_usb.py
```

It reads `private/dev-wifi.env`, probes each Espressif USB Serial/JTAG device with
a nonce, and sends credentials only to firmware that identifies as the WB-12 body.
The firmware tests the network in RAM, reports success only after DHCP, commits
once, and restores the previous network on failure. Join the device's
provisioning SoftAP from a phone or a second interface, not from a Mac's only
internet connection.

## Serial capture

```bash
uv run --locked python tools/capture_serial.py \
  "$SERIAL_PORT" private/device-logs/runtime.log --duration 60
```

Omit `--duration` to run until `Ctrl-C`. Existing logs are protected unless you
pass `--append`. The writer reconnects to the same path after USB resets so panic
and reboot loops stay in one log; `--no-reconnect` keeps a single connection.

## Logic analysis

```bash
tools/bootstrap_sigrok_cli.sh
tools/sigrok.sh runtime-status
tools/sigrok.sh discover --vid 0925 --pid 3881
tools/sigrok.sh doctor --vid 0925 --pid 3881 --location "$ANALYZER_LOCATION" \
  --expected-bcd 0001 --expected-state original
```

Bootstrap and discovery never touch analyzer firmware. `probe` and `capture` are
separate hardware operations: libsigrok uploads `fx2lafw` to RAM and re-enumerates,
writing no EEPROM, so a power cycle restores the analyzer. A 24 MS/s FX2 analyzer
cannot capture the 40 MHz display bus. See `.claude/skills/sigrok-cli/SKILL.md`.

## Local artifacts

`sdkconfig`, `build/`, `.deps/`, `.venv/` and `managed_components/` are local.
`dependencies.lock` and `uv.lock` pin ESP components and Python tools. The
ESP-IDF Python environment lives in
`../../.deps/espressif/python_env/bobe-<version>` and is selected regardless of
the shell's Python.
