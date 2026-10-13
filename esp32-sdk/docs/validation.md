# SDK validation

The SDK supports source checks on a desktop, firmware builds for its target
boards and explicit operation on hardware. A successful build does not qualify a
board's peripherals or acoustics.

## Source and firmware checks

Run the commands in the [SDK development guide](../README.md#development).

## Physical behavior

Use the actual board, intended firmware and current BoBe runtime. Observe:

- Buttons, display, microphone and speaker on that board.
- Pairing and provisioning, permitted capabilities and revoked access.
- Push-to-talk, targeted playback, interruption and reconnect.
- Wi-Fi loss, runtime restart and device reboot without duplicated turns.
- Signed update, failed first boot and rollback.
- Power retention, battery reporting and temperature under representative use.

Preserve device identity and recovery material before any approved flashing or
provisioning operation. Secure Boot and flash-encryption enrollment are separate,
irreversible operations, not part of a routine build.

The WB-12 has dedicated [hardware tooling](../examples/wb12/hil/README.md).
Protocol observations do not prove what came out of a speaker; use the actual
physical input and output. Another board needs its own pin, codec, memory and power
review.
