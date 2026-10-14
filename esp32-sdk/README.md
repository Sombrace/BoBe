# BoBe ESP32 SDK

Build a BoBe surface from an ESP32: a keyboard, a small text display, a button, a
sensor node or a voice companion. Devices own their hardware and real-time I/O;
the BoBe daemon owns conversation, memory, model execution and tools. Device code
does not depend on the daemon's Rust, Swift or TypeScript build. The complete
WonderBoy WB-12 application and its hardware-in-the-loop (HIL) tools are in
[`examples/wb12`](examples/wb12/README.md).

Device components are C11 with C++-compatible APIs. Python runs on the developer's
computer for project creation, certificate and key validation, flashing and
inspection; its `cryptography` package never reaches firmware. On Intel macOS,
current `cryptography` releases build from source and need Rust, a C compiler and
OpenSSL development libraries.

Run the commands below from `esp32-sdk/`. The [main README](../README.md) covers
installing and running the daemon.

## Choose an entry point

| Application | Start here | Requirements |
|---|---|---|
| Keyboard, text screen or application actions | [text_terminal](examples/text_terminal/README.md) | Network; the serial console works as a first UI |
| Sensors and an optional button | [sensor_node](examples/sensor_node/README.md) | Network |
| Wire format in existing firmware | [bobe_protocol](components/bobe_protocol/README.md) | Portable C11/C++; also builds on a desktop |
| Bounded text assembly for a small display | [bobe_text](components/bobe_text/README.md) | Caller-owned buffer; any renderer |
| Full voice companion | [wb12](examples/wb12/README.md) | The documented ESP32-S3 board |

The lightweight client needs no PSRAM, I2S, LVGL or speech models. ESP32,
ESP32-C3, ESP32-C6 and ESP32-S3 are build targets; voice targets ESP32-S3. A build
does not qualify an untested board's peripherals.

## Create a project

With Git, CMake and `uv`, create a project outside this checkout:

```sh
tools/bootstrap.sh
uv run --locked python tools/new_project.py my_keyboard \
  --template text-terminal --target esp32c3 --output ../../my_keyboard
tools/idf.sh -C ../../my_keyboard menuconfig
tools/idf.sh -C ../../my_keyboard build
```

Other templates are `sensor-node`, `protocol` and `wb12`. The generator never
overwrites a destination and copies only a small application, not SDK sources,
credentials, build output or the WB-12 lab tools.

ESP-IDF **6.1** is the default; the WB-12 wrapper defaults to the tested **6.0.2**.
`BOBE_IDF_VERSION` selects either. Tools, Python environments and components stay
in the checkout; existing ESP-IDF users can run `idf.py` directly.

## Enroll and connect

Text, actions and sensors need a daemon with **BodyLink minor 2**; voice-only
bodies and the Swift speech adapter use minor 1.

1. Boot the application and read its persistent device ID from the serial log.
2. Obtain an owner-issued BodyLink claim with the endpoint, CA, client
   certificate and key, device ID and controller epoch (the WB-12 provisioning
   tools use this format).
3. Import it into the application's ignored private directory:

   ```sh
   uv run --locked python tools/import_claim.py /private/path/claim.json \
     --project ../../my_keyboard
   ```

4. Enroll the certificate and grant capabilities in BoBe:

   ```sh
   bobe endpoint enroll DEVICE_ID --kind physical_body --label "Desk keyboard" \
     --certificate-sha256 CERTIFICATE_SHA256 --protocol-min 2 --protocol-max 2 \
     --capabilities '{"text_input":true,"text_output":true,"actions":true,"sensors":true}'
   ```

   A sensor-only device needs `{"sensors":true}`, plus `"actions":true` for its
   button. Reported capabilities never expand owner grants.
5. Configure Wi-Fi in `menuconfig`, rebuild and flash explicitly through ESP-IDF.

The client neither owns Wi-Fi nor erases NVS. Examples use Espressif's network
helper and SNTP; products may supply their own network and trusted time. TLS
verification, pinned enrollment and controller-epoch checks always stay on.

## Use components directly

Point the ESP-IDF Component Manager at a local checkout:

```yaml
dependencies:
  bobe_client:
    path: /path/to/BoBe/esp32-sdk/components/bobe_client
```

or at a pinned commit of this repository:

```yaml
dependencies:
  bobe_client:
    git: https://github.com/Sombrace/BoBe.git
    path: esp32-sdk/components/bobe_client
    version: YOUR_PINNED_COMMIT
```

Sibling components resolve from the same revision. Keep `dependencies.lock`. The
components are not in the public ESP Component Registry.

## Interaction API

Initialize `esp_bobe_client_config_t` with real capabilities and an event callback,
then call `esp_bobe_client_start()` after network and clock setup. Send input with
`esp_bobe_send_text`, `esp_bobe_send_action`, `esp_bobe_report_number` and
`esp_bobe_report_boolean`.

Give each human request its own UUID and keep it until resolved. A successful send
means queued locally; `input.result` reports acceptance and completion. Offline
input is never replayed automatically. Sensor samples update telemetry without
starting a model turn.

Text arrives as bounded UTF-8 `display.text` chunks; `bobe_text_view` handles
sequence gaps, duplicates, new requests and fixed-size storage. Callbacks borrow
data and must not block; copy or queue it for the UI task. Voice bodies can use
the same send APIs on their connection; see
[porting](docs/porting.md#combining-voice-and-other-interactions).

## Development

```sh
uv run --locked python tools/doctor.py --project examples/text_terminal
cmake -S . -B build -DESP_BOBE_SANITIZE=ON
cmake --build build --parallel
ctest --test-dir build --output-on-failure
tools/check.sh
```

`tools/check.sh` needs the prepared WB-12 dependencies and ESP-IDF, and its camera
checks need macOS. The portable C/C++ and Python tests run on Linux without a
board. From the repository root, `scripts/check-bodylink-contract.sh` compares the
Rust, Swift and SDK wire contracts; none of these checks flash a device.

Read [architecture](docs/architecture.md), [porting](docs/porting.md),
[BodyLink 1.2](docs/bodylink-v1.2.md) and [validation](docs/validation.md). WB-12
[snapshot builds](examples/wb12/tools/README.md) record the BoBe commit in
firmware provenance.

## License

First-party code and documentation are MIT-licensed, copyright The BoBe Authors;
see [LICENSE](LICENSE) and the repository's [trademark policy](../TRADEMARKS.md).
Third-party code keeps its own licenses and notices. Factory firmware, recovered
assets and recovery images are not covered by this license and are not in this
repository.
