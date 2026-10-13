# Sensor and optional button node

No microphone, speaker, display, or PSRAM is required. The example reports actual
processor uptime and free internal memory every ten seconds. Replace those two
calls with readings from your sensor driver.

An optional active-low GPIO button sends a named human action. Select its pin
and meaning under **BoBe sensor example** in `menuconfig`; it is disabled by
default. Confirm the board pinout before connecting a switch.

Enroll a minor-2 endpoint with owner permissions `{"sensors":true}`. If the button
is enabled, also grant `"actions":true`. Samples update telemetry without calling
the model; pressing the configured action button starts a conversation turn.

Use `bobe endpoint sensors DEVICE_ID --json` to inspect readings. The daemon
retains fresh readings across device disconnects for a five-minute freshness
window, so a sensor can sleep between reports.

The application uses ESP-IDF's example network helper and SNTP; bring your own
network and time source in a product. Import an owner-issued claim as described
in [Enroll and connect](../../README.md#enroll-and-connect), or use an existing
provisioned NVS claim.
