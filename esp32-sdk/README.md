# BoBe ESP32 SDK

ESP-IDF components and examples for BoBe devices. The device runs its hardware;
the BoBe daemon runs conversation, memory, models and tools. They talk over
[BodyLink](docs/bodylink.md). Install and run the daemon first
([main README](../README.md)), then run the commands below from `esp32-sdk/`.

| Example | What it does | Needs |
|---|---|---|
| [`text_terminal`](examples/text_terminal) | Serial console as keyboard and text display; `/action ID LABEL` presses a button, `/sensor NAME VALUE UNIT` sends a sample | Wi-Fi |
| [`sensor_node`](examples/sensor_node) | Reports uptime and free memory every 10 s; optional button | Wi-Fi |
| [`protocol`](examples/protocol) | Logs encoded BodyLink messages | Any ESP32 |
| [`wb12`](examples/wb12/README.md) | Complete voice body | A WonderBoy WB-12 |

| Component | Use | Runs on |
|---|---|---|
| [`bobe_client`](components/bobe_client/include/esp_bobe_client.h) | Text, actions and sensors over BodyLink | ESP32, -C3, -C6, -S3 |
| [`bobe_text`](components/bobe_text/include/bobe_text_view.h) | Collects `display.text` into a fixed buffer | Any |
| [`bobe_protocol`](components/bobe_protocol/include/bobe_protocol.h) | BodyLink codec and signed-update parser | Any, including a desktop |
| [`bobe_sdk`](components/bobe_sdk/include/esp_bobe_sdk.h) | Complete voice runtime | ESP32-S3 with PSRAM |

## Build and flash an example

You need Git, CMake and [uv](https://docs.astral.sh/uv/).

```sh
tools/bootstrap.sh
tools/idf.sh -C examples/text_terminal set-target esp32c3
tools/idf.sh -C examples/text_terminal menuconfig
tools/idf.sh -C examples/text_terminal build
tools/idf.sh -C examples/text_terminal -p PORT flash monitor
```

`tools/bootstrap.sh` installs ESP-IDF 6.1 into `.deps/`; `BOBE_IDF_VERSION=6.0.2`
installs the version the WB-12 uses. `uv run --locked python tools/doctor.py`
reports what is missing. Set Wi-Fi under **Example Connection Configuration**.
The sensor node's button is off by default; check the pinout before enabling it.
Only the last command writes to the device.

## Create a project

```sh
uv run --locked python tools/new_project.py my_keyboard \
  --template text-terminal --target esp32c3 --output ../../my_keyboard
```

Templates are `text-terminal`, `sensor-node`, `protocol` and `wb12`. The project
uses this checkout by path; `--git URL --ref COMMIT` pins a commit instead (not
for `wb12`). To add the client to existing firmware, declare it in
`main/idf_component.yml` and commit `dependencies.lock`:

```yaml
dependencies:
  bobe_client:
    git: https://github.com/Sombrace/BoBe.git
    path: esp32-sdk/components/bobe_client
    version: COMMIT
```

## Enroll a device

A device connects only with an owner-issued claim and an enrollment that grants
its capabilities. A claim is JSON with `device_id`, `endpoint_uri` (`wss://`),
`trust_kid`, `controller_epoch`, `server_ca_pem`, `client_cert_pem` and
`client_key_pem`; for development,
[`generate_bodylink_pki.sh`](examples/wb12/README.md#provision-and-enroll)
creates one from the `device_id` in the device's serial log. Import the claim
(mode 600) into the project's ignored `private/` directory, rebuild and flash,
then enroll the SHA-256 of the client certificate's DER bytes with the
[grants](docs/bodylink.md#handshake-and-grants) the device may use:

```sh
uv run --locked python tools/import_claim.py /secure/path/claim.json \
  --project examples/text_terminal
bobe endpoint enroll DEVICE_ID --kind physical_body --label "Desk keyboard" \
  --certificate-sha256 CERTIFICATE_SHA256 --protocol-min 2 --protocol-max 2 \
  --capabilities '{"text_input":true,"text_output":true,"actions":true,"sensors":true}'
```

A sensor node needs `{"sensors":true}`, plus `"actions":true` for its button.
`bobe endpoint list` shows grants and what each device reported;
`bobe endpoint revoke DEVICE_ID` cuts a device off. Keep claims out of Git and
logs.

## Use the client

[`text_terminal/main/main.c`](examples/text_terminal/main/main.c) is a complete
client.

- Run one client per chip. Initialize NVS, call `esp_bobe_client_init()` with
  the device's real capabilities, and call `esp_bobe_client_start()` once the
  network is up and the clock is valid. Serialize init, start and stop.
- Send whole entries, not keystrokes, each with a new `esp_bobe_request_id()`
  kept until `input.result` settles it. `ESP_OK` means queued; nothing is sent
  while disconnected or replayed after a reconnect.
- Events are valid only during the callback: copy what you keep, return quickly
  and never call `esp_bobe_client_stop()` there.
- `bobe_text_view` needs a buffer of at least 5 bytes and one task applying
  `display.text`; when full, it drops the oldest characters and sets
  `truncated`.

## Port a voice body

Copy [`examples/wb12`](examples/wb12) and replace `components/bobe_board_port`,
`components/bobe_device_config` and `sdkconfig*.defaults`. `esp_bobe_start()`
returning `ESP_OK` means startup is scheduled, not finished.

| Provide | Contents |
|---|---|
| [`bobe_board.h`](components/bobe_board/include/bobe_board.h) | Power hold, buttons, battery, temperature, backlight, shutdown, and the update key as NUL-terminated PEM whose length includes the NUL |
| [`bobe_board_audio.h`](components/bobe_board/include/bobe_board_audio.h) | Separate I2S controllers: a mono 16 kHz microphone in 32-bit Philips slots (upper 16 bits used) and a mono 24 kHz PCM16 speaker |
| [`bobe_board_display.h`](components/bobe_board/include/bobe_board_display.h) | An RGB565 landscape panel at least 240 px wide with asynchronous transfer completion; the SDK drives LVGL |
| `bobe_device_config.h` | Model, label, hostname prefix, mDNS name, display size and update image limit |

Updates match by model, so give incompatible hardware its own model. For text,
actions or sensors on a voice body, enable `CONFIG_BOBE_ENABLE_TEXT_INPUT`,
`CONFIG_BOBE_ENABLE_TEXT_OUTPUT`, `CONFIG_BOBE_ENABLE_ACTION_INPUT` or
`CONFIG_BOBE_ENABLE_SENSOR_REPORTS`, call `esp_bobe_set_event_handler()` before
`esp_bobe_start()`, and enroll for BodyLink minor 2. Text output reaches your
callback, not the face.

## Check

```sh
cmake -S . -B build -DESP_BOBE_SANITIZE=ON
cmake --build build --parallel
ctest --test-dir build --output-on-failure
tools/check.sh
```

`tools/check.sh` adds the Python, contract and WB-12 host and UI tests; build
`examples/wb12` once first so its managed components exist. These never touch a
device and do not qualify a board. On the real board, back up the flash and
identity, then check enrollment and revocation, buttons, display, microphone,
speaker, reconnects, a daemon restart, a reboot without duplicated turns, a
signed update with rollback, power hold, battery and temperature.

> [!WARNING]
> Secure Boot and flash encryption burn eFuses irreversibly. No build or tool
> here enables them.

## License

MIT, copyright The BoBe Authors; see [LICENSE](LICENSE) and the
[trademark policy](../TRADEMARKS.md). Third-party components keep their own
licenses. Factory firmware, recovered assets and recovery images are not covered
and are not in this repository.
