# BoBe ESP32 SDK

Build a BoBe surface from an ESP32: a keyboard, a small text display, a button,
a sensor node, or a voice companion. Devices own their hardware and real-time
I/O. The BoBe daemon owns conversation, memory, model execution, and tools.

The SDK lives in `esp32-sdk/` beside BoBe's daemon and native app. The complete
WonderBoy WB-12 application, hardware research and hardware-in-the-loop (HIL)
tools are in [`examples/wb12`](examples/wb12/README.md). Device code does not
depend on the daemon's Rust, Swift or TypeScript build.

The device components are C11 with C++-compatible APIs. Python runs on the
developer's computer for project creation, certificate and key validation,
flashing, and hardware inspection. The `cryptography` package supports the
host tools' certificate and signature operations; it is not included in the
device firmware. ESP-IDF also uses Python for its build and flashing tools.
On Intel macOS, current `cryptography` releases build from source and require
Rust, a C compiler, and OpenSSL development libraries.

Run the commands below from `esp32-sdk/`. See the
[main README](../README.md) to install or run the BoBe daemon.

## Choose an entry point

| Application | Start here | Requirements |
|---|---|---|
| Keyboard, text screen, or application actions | [text_terminal](examples/text_terminal/README.md) | Network; serial console works as the first UI |
| Sensors and optional physical button | [sensor_node](examples/sensor_node/README.md) | Network; example reports actual uptime and memory |
| Wire format in existing firmware | [bobe_protocol](components/bobe_protocol/README.md) | Portable C11/C++; also builds on a desktop |
| Bounded text assembly for a small display | [bobe_text](components/bobe_text/README.md) | Caller-owned buffer; any renderer |
| Full voice companion | [wb12](examples/wb12/README.md) | The documented ESP32-S3 board |

The lightweight client needs no PSRAM, I2S, LVGL, or speech models. ESP32,
ESP32-C3, ESP32-C6, and ESP32-S3 are build targets. The voice runtime
targets ESP32-S3. Builds do not qualify physical peripherals on an untested board.

## Create a project

With Git, CMake, and `uv` available, create a consumer outside this checkout:

```sh
tools/bootstrap.sh
uv run --locked python tools/new_project.py my_keyboard \
  --template text-terminal --target esp32c3 --output ../../my_keyboard
tools/idf.sh -C ../../my_keyboard menuconfig
tools/idf.sh -C ../../my_keyboard build
```

Other templates are `sensor-node`, `protocol`, and `wb12`. Existing destinations
are never overwritten. The generator copies a small application, not SDK
sources, credentials, build artifacts, or the WB-12 hardware lab.

ESP-IDF **6.1** is the default development toolchain. The WB-12 wrapper defaults
to the tested **6.0.2** baseline; select either with `BOBE_IDF_VERSION`. Tools,
Python environments, and downloaded components stay in the checkout. Existing
ESP-IDF users can run normal `idf.py` directly.

## Enroll and connect

Text, actions, and sensors require a BoBe daemon with **BodyLink minor 2**.
Voice-only bodies and the Swift speech adapter use minor 1.

1. Boot the application and read its persistent device ID from the serial log.
2. Obtain an owner-issued BodyLink claim with the endpoint, CA, client
   certificate/key, device ID, and controller epoch. The WB-12 provisioning
   tools use this format.
3. Import it into the application's ignored private directory:

   ```sh
   uv run --locked python tools/import_claim.py /private/path/claim.json \
     --project ../../my_keyboard
   ```

4. Enroll its certificate and explicitly grant capabilities in BoBe:

   ```sh
   bobe endpoint enroll DEVICE_ID --kind physical_body --label "Desk keyboard" \
     --certificate-sha256 CERTIFICATE_SHA256 --protocol-min 2 --protocol-max 2 \
     --capabilities '{"text_input":true,"text_output":true,"actions":true,"sensors":true}'
   ```

   A sensor-only device needs `{"sensors":true}`; add `"actions":true` for its
   configured button. Reported capabilities cannot expand owner grants.
5. Configure Wi-Fi in `menuconfig`, rebuild, and explicitly flash through ESP-IDF.

The client does not own Wi-Fi or erase NVS. Examples use Espressif's network
helper and SNTP; products can provide their own network and trusted time source.
TLS verification, pinned client enrollment, and controller-epoch checks stay active.

## Use components directly

ESP-IDF Component Manager can use a local checkout:

```yaml
dependencies:
  bobe_client:
    path: /path/to/BoBe/esp32-sdk/components/bobe_client
```

For a pinned Git revision, replace `REPOSITORY_URL` and `YOUR_PINNED_COMMIT`
with the BoBe repository URL and a commit from this repository:

```yaml
dependencies:
  bobe_client:
    git: REPOSITORY_URL
    path: esp32-sdk/components/bobe_client
    version: YOUR_PINNED_COMMIT
```

Sibling components resolve from the same revision. Keep `dependencies.lock`.
These components are not published in the public ESP Component Registry.

## Interaction API

Initialize `esp_bobe_client_config_t` with real capabilities and an event callback,
then call `esp_bobe_client_start()` after network and clock setup. Use
`esp_bobe_send_text`, `esp_bobe_send_action`, `esp_bobe_report_number`, and
`esp_bobe_report_boolean` for application input.

Each distinct human request needs its own UUID. Retain it until resolved. A
successful send means queued locally; `input.result` reports acceptance and
completion. Offline user input is never replayed automatically. Sensor samples
update telemetry without starting model turns.

Text arrives as bounded UTF-8 `display.text` chunks. `bobe_text_view` handles
sequence gaps, duplicates, new requests, and fixed-size display storage. Callbacks
borrow data and must stay nonblocking; copy or queue it for the UI task.

Voice bodies can use the same send APIs over their existing connection; see
[porting](docs/porting.md#combining-voice-and-other-interactions).

## Development

```sh
uv run --locked python tools/doctor.py --project examples/text_terminal
cmake -S . -B build -DESP_BOBE_SANITIZE=ON
cmake --build build --parallel
ctest --test-dir build --output-on-failure
tools/check.sh
```

The full `tools/check.sh` path needs the prepared WB-12 dependencies and ESP-IDF.
Camera checks also need macOS. The portable C/C++ commands and SDK Python tests
can run on Linux without a board.

From the BoBe repository root, `scripts/check-bodylink-contract.sh` compares
the Rust, Swift and SDK wire contracts. SDK source checks are separate from
the daemon and Mac builds; they do not flash devices.

Read [architecture](docs/architecture.md), [porting](docs/porting.md),
[BodyLink 1.2](docs/bodylink-v1.2.md), and [validation](docs/validation.md).

## Source snapshots

SDK [snapshot builds](examples/wb12/tools/README.md) select the committed
`esp32-sdk` subtree. Firmware provenance records the BoBe commit.

## License

First-party code and documentation are licensed under the [MIT License](LICENSE),
copyright The BoBe Authors. Third-party code and dependencies retain their own
licenses and notices.

Factory firmware, recovered assets, and external recovery images are not covered
by the project's MIT license and are not distributed in this repository.
