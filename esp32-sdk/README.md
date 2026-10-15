# BoBe ESP32 SDK

ESP-IDF components and examples for BoBe devices: a voice body, a keyboard or
text display, buttons and sensors. A device owns its hardware and real-time
input and output. The BoBe daemon owns conversation, memory, models and tools,
and the device reaches it over [BodyLink](docs/bodylink.md). Install and run the
daemon first ([main README](../README.md)). Device code is C11 with
C++-compatible headers; Python runs only on your computer. Run the commands
below from `esp32-sdk/`.

## Start here

| Example | What it does | Needs |
|---|---|---|
| [`examples/text_terminal`](examples/text_terminal) | Serial console as keyboard and text display, plus action and sensor commands | Wi-Fi |
| [`examples/sensor_node`](examples/sensor_node) | Reports uptime and free memory every 10 s; optional button | Wi-Fi |
| [`examples/protocol`](examples/protocol) | Logs encoded BodyLink messages; no network | Any ESP32 |
| [`examples/wb12`](examples/wb12/README.md) | Complete voice body | A WonderBoy WB-12 |

| Component | Use | Header |
|---|---|---|
| `bobe_client` | Text, actions and sensors over one BodyLink connection | `esp_bobe_client.h` |
| `bobe_text` | Collects `display.text` into a fixed buffer for any renderer | `bobe_text_view.h` |
| `bobe_protocol` | Portable BodyLink codec and signed-update parser; also builds on a desktop | `bobe_protocol.h` |
| `bobe_sdk` | Complete ESP32-S3 voice runtime | `esp_bobe_sdk.h` |

The other components make up the voice runtime, which needs an ESP32-S3 with
PSRAM. `bobe_client` needs no PSRAM, audio, display or speech models and builds
for ESP32, ESP32-C3, ESP32-C6 and ESP32-S3. `bobe_protocol` builders write into
your buffer and return `BOBE_PROTOCOL_TOO_LARGE` when the JSON does not fit. A
build does not prove a board works.

## Build and flash an example

You need Git, CMake and [uv](https://docs.astral.sh/uv/).

```sh
tools/bootstrap.sh
tools/idf.sh -C examples/text_terminal set-target esp32c3
tools/idf.sh -C examples/text_terminal menuconfig
tools/idf.sh -C examples/text_terminal build
tools/idf.sh -C examples/text_terminal -p PORT flash monitor
```

`tools/bootstrap.sh` installs ESP-IDF 6.1 under `.deps/`, never globally;
`BOBE_IDF_VERSION=6.0.2` selects the version the WB-12 uses, and `idf.py` works
directly too. `uv run --locked python tools/doctor.py` reports what is missing.
On Intel Macs, `cryptography` builds from source and needs Rust, a C compiler
and OpenSSL headers.

Set Wi-Fi under **Example Connection Configuration**; the sensor node's button
(off by default; check the pinout first) is under **BoBe sensor example**. Only
the last command flashes. The examples use Espressif's example Wi-Fi helper and
SNTP; a product brings its own network and the trusted clock TLS needs. In the
text terminal, type a message and press Enter; `/action ID LABEL` sends a button
press and `/sensor NAME VALUE UNIT` a sample.

## Create a project

```sh
uv run --locked python tools/new_project.py my_keyboard \
  --template text-terminal --target esp32c3 --output ../../my_keyboard
tools/idf.sh -C ../../my_keyboard build
```

Templates are `text-terminal`, `sensor-node`, `protocol` and `wb12`. The
generator copies only the small application, never SDK sources, credentials or
build output, and never overwrites a directory. The project uses this checkout
by path; `--git URL --ref COMMIT` pins a commit of this repository instead (not
for `wb12`). To add the client to existing firmware, declare it in
`main/idf_component.yml`:

```yaml
dependencies:
  bobe_client:
    git: https://github.com/Sombrace/BoBe.git
    path: esp32-sdk/components/bobe_client
    version: COMMIT
```

Use `path:` alone for a local checkout. Sibling components resolve from the same
revision; commit `dependencies.lock`. The SDK is not in the ESP Component Registry.

## Enroll and connect

A device connects only with an owner-issued claim and an enrollment that grants
its capabilities. Text, actions and sensors need BodyLink minor 2.

1. Boot the application and copy its `device_id` from the serial log.
2. Get a claim: a JSON file with `device_id`, `endpoint_uri` (`wss://`),
   `trust_kid`, `controller_epoch`, `server_ca_pem`, `client_cert_pem` and
   `client_key_pem`. For development, `examples/wb12/tools/generate_bodylink_pki.sh`
   creates one along with the daemon's TLS settings; see
   [provision and enroll](examples/wb12/README.md#provision-and-enroll).
3. Import it into the application's ignored `private/` directory (the claim must
   be mode 600):

   ```sh
   uv run --locked python tools/import_claim.py /secure/path/claim.json \
     --project examples/text_terminal
   ```

4. Enroll the client certificate (the SHA-256 of its DER bytes) with the
   capabilities the device may use:

   ```sh
   bobe endpoint enroll DEVICE_ID --kind physical_body --label "Desk keyboard" \
     --certificate-sha256 CERTIFICATE_SHA256 --protocol-min 2 --protocol-max 2 \
     --capabilities '{"text_input":true,"text_output":true,"actions":true,"sensors":true}'
   ```

   A sensor node needs `{"sensors":true}`, plus `"actions":true` for its button.
   [BodyLink](docs/bodylink.md#handshake-and-grants) explains minors and grants.
5. Rebuild and flash. `bobe endpoint list` shows grants and what the device
   reported; `bobe endpoint revoke DEVICE_ID` cuts it off.

The client never configures Wi-Fi or erases NVS. Keep claims out of Git and logs.

## Interaction API

[`examples/text_terminal/main/main.c`](examples/text_terminal/main/main.c) is a complete client.

- Run one client per chip. Initialize NVS, then call `esp_bobe_client_init()`
  with the device's real capabilities and its event handler; a NULL `device_id`
  or `credentials` uses the device ID or claim stored in NVS. Call
  `esp_bobe_client_start()` after the network is up and the clock is valid, and
  serialize init, start and stop.
- Send whole entries, not keystrokes, with `esp_bobe_send_text`,
  `esp_bobe_send_action`, `esp_bobe_cancel_input`, `esp_bobe_report_number` and
  `esp_bobe_report_boolean`. `esp_bobe_request_id()` fills a 37-byte buffer; keep
  the ID until `input.result` settles it.
- `ESP_OK` means queued on the device. Nothing is sent while disconnected,
  replayed after a reconnect or retried under a new ID; ask the person first.
- An event is valid only during the callback: copy what you keep, return quickly
  and never call `esp_bobe_client_stop()` from it.
- `bobe_text_view` keeps response text in a buffer you own of at least 5 bytes.
  Call `bobe_text_view_init()` once, then `bobe_text_view_apply()` for each
  `display.text` from one task, and render `view.buffer` when it returns
  `BOBE_TEXT_APPLIED`. When full it drops the oldest whole characters and sets
  `truncated`.

## Port a voice body

Start from [`examples/wb12`](examples/wb12) and replace its
`components/bobe_board_port`, `components/bobe_device_config` and
`sdkconfig*.defaults`. The project `CMakeLists.txt` includes
`cmake/esp_bobe_sdk.cmake` before ESP-IDF's `project.cmake`. `main` requires
`bobe_sdk` and the board port and calls `esp_bobe_start()` once; it holds board
power and schedules startup, so `ESP_OK` means scheduled, not ready.

| Provide | Contents |
|---|---|
| `bobe_board.h` | Early power hold, buttons, battery, temperature, backlight, shutdown, and the public update key as NUL-terminated PEM (length includes the NUL) |
| `bobe_board_audio.h` | Two I2S controllers: a 16 kHz microphone read as 32-bit Philips slots (upper 16 bits used) and a 24 kHz PCM16 speaker; the port sets pins, clock inversion, channel and slot width |
| `bobe_board_display.h` | An RGB565 panel with asynchronous transfer completion, its orientation and byte order; the SDK owns LVGL and flushing |
| `bobe_device_config.h` | Model, label, hostname prefix, mDNS name, display size and update image limit |

The first three headers are in [`components/bobe_board/include`](components/bobe_board/include).
Audio is mono; a shared full-duplex codec needs its own driver. The face needs a
landscape panel at least 240 px wide. Give incompatible hardware its own model
string, because updates match by model.

To add text, actions or sensors to a voice body, enable
`CONFIG_BOBE_ENABLE_TEXT_INPUT`, `CONFIG_BOBE_ENABLE_TEXT_OUTPUT`,
`CONFIG_BOBE_ENABLE_ACTION_INPUT` or `CONFIG_BOBE_ENABLE_SENSOR_REPORTS`, call
`esp_bobe_set_event_handler()` before `esp_bobe_start()`, and use the client send
functions over the voice connection. Text output reaches your callback, not the
face. The device then needs a minor-2 enrollment with those grants.

## Checks and hardware validation

```sh
cmake -S . -B build -DESP_BOBE_SANITIZE=ON
cmake --build build --parallel
ctest --test-dir build --output-on-failure
tools/check.sh
```

The CMake build runs the portable C and C++ tests on Linux or macOS.
`tools/check.sh` adds the Python tests, type checks, the device contract test and
the WB-12 host and UI tests; build `examples/wb12` once first so its managed
components exist. From the repository root, `scripts/check-bodylink-contract.sh`
compares the C, Rust and Swift BodyLink constants. None of these flash a device.

Passing checks do not qualify a board. On the real board, with the intended
firmware and a current daemon, try enrollment and revocation, every button, the
display, microphone and speaker, push-to-talk, reconnects after Wi-Fi loss, a
daemon restart and a reboot without duplicated turns, a signed update with
rollback, power hold, battery level and temperature. Back up the flash and
identity first. Secure Boot and flash encryption burn eFuses irreversibly and
are never part of a build. The WB-12 has [HIL tools](examples/wb12/hil/README.md).

## License

MIT, copyright The BoBe Authors; see [LICENSE](LICENSE) and the repository's
[trademark policy](../TRADEMARKS.md). Third-party components keep their own
licenses and notices. Factory firmware, recovered assets and recovery images are
not covered by this license and are not in this repository.
