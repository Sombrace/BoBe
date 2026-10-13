# bobe_client

An ESP-IDF client for BoBe text input/output, action buttons, and sensor telemetry.
Public API: `esp_bobe_client.h`. It uses the shared `bobe_transport` connection
and portable `bobe_protocol` types.

Initialize NVS, choose explicit capabilities, set an event callback, and initialize
the client. Start it after networking and certificate-validating time are ready.
It does not configure Wi-Fi, erase storage, or send input while disconnected.

One client runs per MCU. Lifecycle calls are serialized by the application.
Start/stop are repeatable; bounded transport resources live for the firmware's
lifetime. Callbacks run on transport tasks and must not block or call stop.
Message pointers are borrowed only during the callback.

To use the same send APIs on the full voice runtime, see
[porting](../../docs/porting.md#combining-voice-and-other-interactions).

See [text_terminal](../../examples/text_terminal/README.md),
[sensor_node](../../examples/sensor_node/README.md), and the
[protocol contract](../../docs/bodylink-v1.2.md).
