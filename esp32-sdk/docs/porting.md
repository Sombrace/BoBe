# Porting the BoBe ESP32 SDK

## Existing ESP32 firmware

Add `bobe_client` through Component Manager. Your application owns NVS setup,
networking, a certificate-validating clock, hardware drivers, and rendering.
Initialize the client with its real capabilities and callback, then start it.

Use `text_input` for completed keyboard entries, `actions` for human button/menu
choices, `sensors` for observations, and `text_output` for a renderer. Enroll only
the permissions that device needs. A new claim in `hello` cannot grant itself
permission to start a conversation.

Callbacks borrow their data. Keep them nonblocking; copy/queue messages for the
application's UI task. `bobe_text_view` provides bounded UTF-8 state for an OLED,
LCD, LVGL label, or serial console. It owns no peripheral or event loop.

The lightweight client works without audio, a display driver, or PSRAM. It keeps
one endpoint/transport for the firmware lifetime. Stop/start are repeatable;
lifecycle calls must be serialized, and stop must not run inside a callback.
There is no offline user-input replay.

## Voice body ports

The full runtime targets ESP32-S3. Start from `examples/wb12`, replace
its board component and ESP-IDF defaults, and include the SDK CMake entry point:

```cmake
include("${ESP_BOBE_SDK_PATH}/cmake/esp_bobe_sdk.cmake")
include($ENV{IDF_PATH}/tools/cmake/project.cmake)
project(my_bobe_body)
```

The application supplies `bobe_board_port` and `bobe_device_config`; `main`
requires `bobe_sdk` and the board implementation. Call `esp_bobe_start()` once.
It retains board power before scheduling startup. `ESP_OK` means startup was
scheduled, not that peripherals or the network are ready.

Implement the headers in `components/bobe_board/include`:

- `bobe_board.h`: early power, buttons, battery, temperature, backlight, shutdown,
  and the public OTA verification key.
- `bobe_board_audio.h`: I2S controllers, pins, clocks, slots, and channel selection.
- `bobe_board_display.h`: RGB565 panel setup, completion callbacks, rotation,
  and byte ordering.

Device configuration supplies the model, label, hostname prefix, mDNS instance,
logical display geometry, and OTA image-size limit. Use a distinct model for
incompatible hardware. The application owns partitions, build number, credentials,
and update keys; the portable OTA parser receives its expected model explicitly.

Voice audio is mono PCM: microphone 16 kHz in the upper 16 bits of a
32-bit Philips I2S word, and speaker 24 kHz/16-bit data on another I2S controller.
A shared full-duplex codec or other sample formats need a real driver port.
Do not treat accepted DMA buffers or a codec write return as completed playback.

The face renderer assumes a landscape RGB565 panel at least 240 pixels in each
dimension. The voice scheduler and queues assume dual-core S3 with PSRAM.

## Combining voice and other interactions

Enable the desired `BOBE_ENABLE_TEXT_INPUT`, `BOBE_ENABLE_TEXT_OUTPUT`,
`BOBE_ENABLE_ACTION_INPUT`, or `BOBE_ENABLE_SENSOR_REPORTS` Kconfig options.
Register `esp_bobe_set_event_handler()` before starting the voice runtime, and
use `esp_bobe_send_text`, `esp_bobe_send_action`, and the sensor APIs. They share
its existing transport. Supply a renderer for enabled text output; the generic
callback does not change the WB-12 face renderer automatically.

Such a device needs a minor-2 enrollment granting the additional capabilities.
The default WB-12 configuration uses the minor-1 voice profile.

## Qualification

Build the application outside the SDK tree, run the portable and protocol checks,
and exercise real enrollment, reconnect, cancellation, and output backpressure.
On hardware, verify power hold, input timing, microphone privacy, speaker drain,
LCD completion, Wi-Fi, and rollback; see [validation](validation.md).

Private signing keys and provisioning credentials remain outside source
packages; a board returns only its public update verification key to the runtime.
