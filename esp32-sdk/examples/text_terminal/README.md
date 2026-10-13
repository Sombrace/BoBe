# Text, buttons, and sensors without an audio board

This application turns an ESP32 serial monitor into a BoBe keyboard and text
display. It also demonstrates action buttons and manually entered sensor samples.
It needs no display driver, I2S device, LVGL, ESP-SR, or PSRAM.

The SDK exposes the same event callback for a small LCD/OLED and the same send
functions for a physical keyboard, GPIO button, or sensor driver. Submit a
completed text entry; individual keystrokes stay on the device.

```sh
tools/idf.sh -C examples/text_terminal set-target esp32c3
tools/idf.sh -C examples/text_terminal menuconfig
tools/idf.sh -C examples/text_terminal build
```

Configure Wi-Fi under **Example Connection Configuration**. The example uses
Espressif's example networking helper; applications can bring their own network
connection.

Device enrollment requires an owner-approved endpoint and its TLS identity.
Use an existing provisioned NVS claim or import a claim with
`tools/import_claim.py --project examples/text_terminal /private/path/claim.json`.
It writes the credentials and provisioned device ID to the ignored
`private/` directory. BoBe must support BodyLink minor 2 and the endpoint must
be enrolled with protocol range 2–2 (or a range containing 2).

After connecting, type a message and press Enter. Other commands are:

- `/action menu.next Next page`: submit a human button/interaction event.
- `/sensor temperature 22.5 C`: publish a manually entered sample. Replace this
  call with a real sensor driver in your application.

Sensor samples update device telemetry; they do not start a model turn.
Watch `input.result` events to distinguish queued input from accepted or completed
work.

The normal build contains no credentials. Hardware flashing is an explicit
operator action through ESP-IDF after reviewing the pin/connection configuration.
