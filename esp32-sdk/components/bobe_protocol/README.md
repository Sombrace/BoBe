# bobe_protocol

Portable C11/C++ BodyLink encoding, decoding, capability declarations, media
framing, and signed OTA format validation. No board, RTOS, NVS, or display header
is required. The only JSON dependency is the pinned cJSON component.

Use `bobe_protocol.h` for wire types and builders. `bobe_update_manifest.h` takes
an explicit expected model and maximum image size; `bobe_update_offer.h` delegates
signature verification to a caller-provided verifier.

Buffers are caller-owned. Builders return a status when input or escaped output
exceeds limits. Incoming control JSON has a 4096-byte and sixteen-level nesting
bound. Do not treat syntactically valid messages as device authorization; the
daemon must enforce enrollment grants and request ownership.

See [BodyLink 1.2](../../docs/bodylink-v1.2.md). Native builds and C++ linkage
checks are in the [SDK development commands](../../README.md#development).
