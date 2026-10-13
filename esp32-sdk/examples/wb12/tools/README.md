# Developer tools

All Python tools run in the project environment with `uv`; nothing needs to be
installed globally.

| Tool | Purpose |
|---|---|
| `capture_serial.py` | Write timestamped live ESP serial output to a durable log |
| `bobe_image.py` | Validate ESP32-S3 app descriptors, manifests, and OTA selection |
| `flash_app_verified.sh` | Manifest-gated `ota_0` app flash with read-back verification |

`flash_app_verified.sh` flashes the board on the serial port that you give. It
uses the ESP32-S3 native USB-Serial/JTAG reset sequence to enter the ROM loader
and prints the board's MAC. It verifies the active `ota_0` selection, security
state, app descriptor and signed clean-source manifest. It writes only the
application partition at `0x20000` and verifies the written bytes before
restarting. `bobe_hil.sh device flash` is the parameterized
equivalent: it takes the image, optional manifest, offset, rollback image and
boot observation window from the caller. Manual BOOT-button timing is not
required when the native reset path is responsive; the EN fixture remains the
recovery path.

## Firmware development

| Tool | Purpose |
|---|---|
| `bootstrap_idf.sh` | Prepare ESP-IDF in the SDK checkout; default baseline 6.0.2 |
| `idf.sh` | Run the project-local ESP-IDF CLI environment |
| `check.sh` | Run host tests and build the ESP32-S3 firmware without flashing |
| `build_bobe_snapshot.sh` | Build normal BoBe firmware from the committed Git snapshot and emit provenance hashes |
| `bobe_hil.sh` | External lab tool: drive the WB-12 running normal firmware, Pico fixture, router impairment, observer, scenarios, and UI/media evidence |
| `bobe_capture.sh` | Build/sign/run the camera CLI; use this TCC-owning wrapper, not its ignored `.tmp` binary |
| `bootstrap_sigrok_cli.sh` | Build a hash-pinned native arm64 `sigrok-cli` runtime without Rosetta |
| `sigrok.sh` | Discover FX2 analyzers and perform bounded capture/inspection/conversion |
| `bootstrap_pico_fixture.sh` | Fetch the pinned official Pico 2 SDK/toolchain |
| `build_pico_fixture.sh` | Build the semantic open-drain fixture firmware |
| `generate_provisioning_credentials.sh` | Atomically create or explicitly rotate a versioned private Security-2 credential bundle |
| `generate_bodylink_pki.sh` | Create development BodyLink CA, server/device certificates and claim payload |
| `provision_bobe.sh` | Validate a matching private claim and provision through the Security-2 SoftAP |

Typical workflow:

```bash
tools/bootstrap_idf.sh
tools/check.sh
```

The BoBe snapshot build archives only the committed `esp32-sdk` subtree and
builds a normal product profile (`fallback`, `ptt` or `edge`; default `edge`).
Its lock records the combined repository's source commit, image size/hash, and both ELF hashes in
`build-bobe-snapshot/bobe-snapshot.lock`. The lock and adjacent application
manifest travel with the build. `device flash` uses the application manifest
to bind boot identity:

```bash
tools/build_bobe_snapshot.sh --profile edge
tools/bobe_hil.sh device flash \
    --image build-bobe-snapshot/bobe_wb12.bin \
    --rollback-image build-ondevice-reviewed/bobe_wb12.bin
```

The lock is provenance data; `device flash` reports whether a supplied/adjacent
manifest and the selected OTA slot allowed the observed boot to be bound to the
written image.

Every tool here runs unprivileged in the repository. Nothing installs into the
system, and no tool needs `sudo`.

## Generic logic analysis

The sigrok wrapper is a reusable analyzer tool. The current toy is sealed and
its 40 MHz display bus exceeds the connected
24 MS/s analyzer.

Bootstrap and read-only discovery do not touch analyzer firmware:

```bash
tools/bootstrap_sigrok_cli.sh
tools/sigrok.sh runtime-status
tools/sigrok.sh discover --vid 0925 --pid 3881
```

Set `ANALYZER_LOCATION` to the selected analyzer's IOKit location from discovery,
then check its identity:

```bash
tools/sigrok.sh doctor \
  --vid 0925 --pid 3881 \
  --location "$ANALYZER_LOCATION" \
  --expected-bcd 0001 \
  --expected-state original
```

`probe` and `capture` are separate hardware operations. When the analyzer is in
its original state, libsigrok uploads `fx2lafw` into RAM and re-enumerates USB.
No EEPROM is written; power cycling restores the original state. Read
`.claude/skills/sigrok-cli/SKILL.md` for setup references.

BodyLink PKI generation requires the intended stable device UUID explicitly:

```bash
tools/generate_bodylink_pki.sh private/bodylink-pki <device-uuid>
```

Security-2 credentials never rotate implicitly:

```bash
tools/generate_provisioning_credentials.sh --create --device-id <device-uuid>
tools/generate_provisioning_credentials.sh --rotate --device-id <device-uuid>
```

Rotation archives the complete prior C/private-manifest pair before publishing
the replacement directory. Live provisioning writes a private JSON receipt
after device preflight, claim commit, Wi-Fi configuration and Wi-Fi apply, so a
late failure is resumable and never reported as an all-or-nothing unknown.

For a local test device, put the test network in the ignored
`private/dev-wifi.env`:

```bash
BOBE_WIFI_SSID='<network>'
BOBE_WIFI_PASSWORD='<password>'
tools/build_dev_wifi.sh
```

The generated SDKCONFIG, ELF, and application image remain ignored and contain
the credential. Flash the development image only long enough to confirm the
NVS-backed connection, then overwrite the application slot with a normal
product image without erasing NVS.

Normal production firmware can change Wi-Fi directly over its USB-C cable,
without reflashing:

```bash
uv run --group hil --locked python tools/configure_bobe_usb.py
```

The client reads `private/dev-wifi.env`, probes every attached Espressif USB
Serial/JTAG candidate with a nonce, and sends credentials only to the firmware
that identifies itself as the WB-12 body. Firmware tests the network in RAM,
reports success only after DHCP, commits once, and restores the prior network
after authentication, AP, or timeout failure.

Do not switch the development Mac away from its internet connection to join
the isolated device SoftAP. Use a phone, secondary network interface, or a
separate provisioning client.

The generated `sdkconfig`, `build/`, `.deps/`, `.venv/`, and
`managed_components/` directories are local artifacts. Exact ESP component
versions and Python tooling dependencies are retained in `dependencies.lock`
and `uv.lock`.

Set `SERIAL_PORT` to the connected board's serial port, then capture live
firmware output with host timestamps:

```bash
uv run --locked python tools/capture_serial.py \
  "$SERIAL_PORT" \
  private/device-logs/runtime.log \
  --duration 60
```

Omit `--duration` for a continuous writer and stop it with `Ctrl-C`. Existing
logs are protected by default; pass `--append` only when continuing the same
capture. The writer reconnects to the same serial path after USB resets by
default, keeping panic and reboot loops in one log. Use `--no-reconnect` for a
single connection.

`check.sh` always starts from fresh build-local configuration and validates
host tests, sanitizers, logical UI references, Pico fixture firmware, macOS
capture tooling, fallback firmware, a Security-2 push-to-talk profile, and the
Security-2 plus ESP-SR edge profile. Every firmware profile is compile-only,
and each one must retain the production physical button event log that external
scenarios observe.

Named narrow gates:

```bash
tools/check.sh core-policy
tools/check.sh ui
tools/check.sh fixture
tools/check.sh capture
```

Inspect current hardware availability without touching hardware:

```bash
tools/bobe_hil.sh doctor
tools/bobe_hil.sh device status
tools/bobe_hil.sh scenario list
tools/bobe_hil.sh scenario run physical-controls --dry-run
tools/bobe_hil.sh scenario run voice-roundtrip --mock --junit .tmp/mock.xml
tools/bobe_hil.sh scenario run ui-golden --live --junit .tmp/ui.xml
```

The scenario runner dispatches the steps declared in `hil/scenarios/*.toml`
directly to the local device, fixture, network, media, camera and BodyLink
implementations. `--dry-run` prints the plan and exits `0` with status
`planned`; failures exit `1`; scenarios needing absent hardware exit `3` with
status `blocked`. Mock mode exercises all physical steps and cleanup with a
`simulation` claim and cannot satisfy a physical requirement. Physical
controls, network faults, calibrated screen capture and acoustic evidence stay
unavailable until the fixtures described in `hil/README.md` are connected.

The ESP-IDF bootstrap uses `uv` and installs its Python environment under
`../../.deps/espressif/python_env/bobe-<version>`. The wrapper selects this environment
regardless of the shell’s default Python version; no global packages are needed.
