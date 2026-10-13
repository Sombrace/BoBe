# Changelog

## Unreleased

- Move the SDK into the BoBe repository under `esp32-sdk/`.
- Use the combined repository for pinned component dependencies and snapshot
  provenance.

## 0.2.0

- Rename the firmware project to esp-bobe and retain WB-12 as a complete example.
- Extract portable protocol/OTA parsing, bounded text-view state, ESP-IDF
  identity/storage, and a lightweight client.
- Add BodyLink minor-2 text input/output, action buttons, sensor telemetry,
  explicit enrollment grants, request replay protection, and cancellation.
- Add project generation, private-claim import, environment diagnostics,
  component-manager manifests, and native/firmware checks.
- Keep minor-1 voice and speech-adapter compatibility.

The component API is pre-1.0. Pin a revision and review changes before upgrading.
