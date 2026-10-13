# BodyLink V1 Wire Contract

Minor version: 1
Protocol: `bobe.body.v1`
Authority: this example's [`docs/architecture.md`](architecture.md) for the
device and BoBe's root [`docs/architecture.md`](../../../../docs/architecture.md)
for runtime routing

This is the contract shared by the WB-12 firmware and Rust BodyLink gateway.
BodyLink is separate from BoBe's `bobe.voice.v2` WebSocket.

## Connection

```text
wss://<runtime-or-gateway>/body/v1
Sec-WebSocket-Protocol: bobe.body.v1
```

Requirements:

- one connection per body;
- authenticated TLS;
- device-scoped authorization, never the broad daemon bearer token;
- text WebSocket messages for JSON control;
- binary WebSocket messages for media;
- 4096-byte maximum incoming control message;
- 10-second WebSocket ping interval;
- a fresh hello/welcome handshake after reconnect;
- no audio replay after reconnect.

The Security-2 claim stores the private server CA, device client certificate,
development client key, endpoint URI, trust-key ID and controller epoch.
Production hardware should generate the private key locally or use the
ESP32-S3 Digital Signature peripheral.

The development firmware can use `ws://` only when the explicit insecure
Kconfig option is enabled. Production must use WSS and mutual TLS.

## Identity and versioning

The first device message is `hello`:

```json
{
  "type": "hello",
  "protocol_major": 1,
  "protocol_minor": 1,
  "device_id": "wb12-a1b2c3d4e5f6",
  "boot_id": "7f81d394f3f945de873c18a0ea40db93",
  "firmware": "0.2.0",
  "hardware": "wonderboy-wb12",
  "capabilities": {
    "display": {
      "width": 280,
      "height": 240,
      "face_schema": 1
    },
    "microphone": true,
    "speaker": true,
    "battery": true,
    "aec": false,
    "wake_word": false
  },
  "limits": {
    "max_control_bytes": 4096,
    "max_playback_credit_ms": 300
  }
}
```

The gateway replies:

```json
{
  "type": "welcome",
  "protocol_major": 1,
  "protocol_minor": 1,
  "session_id": "body-session-id",
  "connection_generation": 2,
  "heartbeat_interval_ms": 10000,
  "controller_epoch": 1
}
```

Rules:

- a major-version mismatch rejects the connection;
- the firmware advertises minor 1 and the gateway accepts it because it supports
  that minor; a minor it does not support rejects the connection;
- unknown optional JSON fields are ignored;
- firmware minor 1 ignores unknown message types;
- `boot_id` changes on every device boot;
- `controller_epoch` fences stale runtime instances.
- the body requires the runtime epoch to equal the authenticated claim epoch;
  changing authority requires a newer Security-2 claim.

## Control envelope

Minor 1 guarantees:

- `type`;
- `protocol_major`;
- fields with explicit units such as `_ms`, `_us`, `_hz`, or `_samples`.

Control messages are never mixed into PCM payloads.

After a closed playback stream physically drains, the body sends correlated
`audio.playback.drained`. A timeout or local playback abort instead sends
`audio.playback.failed` with a reason. The hub retains the voice lease and
global turn-admission permit until one truthful terminal outcome arrives.

### Authoritative route

Every conversational command carries enough identity for the daemon to reject
stale work:

- stable enrolled `device_id`;
- boot ID and daemon-owned connection generation;
- device-created `request_id`;
- daemon-created `lease_id` and `turn_id`;
- media `stream_id`;
- runtime controller epoch;
- an explicit TTL or monotonic deadline.

The gateway binds the authenticated client certificate to the enrolled
device ID, keeps one atomic lease lifecycle, and validates the complete route on
every control and media operation. Swift may provide fallback FluidAudio STT,
but only as a correlated adapter. It must not open another persistent
`/voice/stream`, generate turn IDs, own routing, or broadcast private body
token/tool events into the Mac SSE stream.

The Swift adapter receives the daemon-owned route and never supplies one from
device input.

## Push-to-talk capture

The action button does not begin network audio until the gateway grants the
capture lease.

Device:

```json
{
  "type": "capture.request",
  "protocol_major": 1,
  "request_id": "device-request-id",
  "stream_id": 174321,
  "trigger": "action_button",
  "codec": "pcm_s16le",
  "sample_rate_hz": 16000,
  "channels": 1,
  "frame_duration_ms": 20
}
```

Gateway:

```json
{
  "type": "capture.granted",
  "protocol_major": 1,
  "request_id": "device-request-id",
  "stream_id": 174321,
  "lease_id": "daemon-lease-uuid",
  "turn_id": "voice_daemon_turn_id",
  "lease_ttl_ms": 120000
}
```

`lease_ttl_ms` is the initial watchdog window, not a fixed deadline for the
whole turn. The daemon advances bounded phase deadlines when it receives
capture, transcription, generation, decode, credit, or drain progress, with a
hard ten-minute ceiling so a stalled endpoint cannot own admission forever.
The body validates this daemon-owned field but does not retain it; its
independent local safety ceiling remains ten minutes.

or:

```json
{
  "type": "capture.denied",
  "protocol_major": 1,
  "request_id": "device-request-id",
  "stream_id": 174321,
  "reason": "another_endpoint_owns_voice_lease"
}
```

If the action button is released before the grant arrives, the device sends:

```json
{
  "type": "capture.cancel",
  "protocol_major": 1,
  "request_id": "device-request-id",
  "stream_id": 174321
}
```

Cancellation is idempotent. An already denied or released request does not
disconnect the body; a matching granted-but-not-finished route releases its
admission permit and lease.

After grant, the body starts I2S and sends:

```json
{
  "type": "audio.capture.open",
  "protocol_major": 1,
  "lease_id": "daemon-lease-uuid",
  "turn_id": "voice_daemon_turn_id",
  "stream_id": 174321,
  "codec": "pcm_s16le",
  "sample_rate_hz": 16000,
  "channels": 1,
  "frame_duration_ms": 20
}
```

On release:

```json
{
  "type": "audio.capture.close",
  "protocol_major": 1,
  "lease_id": "daemon-lease-uuid",
  "turn_id": "voice_daemon_turn_id",
  "stream_id": 174321,
  "reason": "button_released"
}
```

Microphone clocks are disabled outside granted capture unless the optional
WakeNet monitor is explicitly active.

## Playback

Gateway:

```json
{
  "type": "audio.playback.open",
  "protocol_major": 1,
  "lease_id": "daemon-lease-uuid",
  "turn_id": "voice_daemon_turn_id",
  "stream_id": 32012,
  "codec": "pcm_s16le",
  "sample_rate_hz": 24000,
  "channels": 1,
  "frame_duration_ms": 20
}
```

The body first confirms that its I2S path and bounded queue are ready:

```json
{
  "type": "audio.playback.ready",
  "protocol_major": 1,
  "lease_id": "daemon-lease-uuid",
  "turn_id": "voice_daemon_turn_id",
  "stream_id": 32012,
  "credit_ms": 300
}
```

The gateway must not send PCM before this acknowledgement. It then sends
speaker PCM binary frames.

The body scales PCM outside ISR context and preloads the I2S DMA ring before
enabling the channel. A stream does not begin from empty DMA descriptors.

The first ready credit describes the empty stream's receive capacity, up to
300 ms. The gateway retains that bound for the stream; later credit reports
cannot enlarge it. As frames physically leave the I2S DMA path, the body
replenishes free capacity:

```json
{
  "type": "audio.playback.ack",
  "protocol_major": 1,
  "lease_id": "daemon-lease-uuid",
  "turn_id": "voice_daemon_turn_id",
  "stream_id": 32012,
  "played_samples": 4800,
  "credit_ms": 80
}
```

`played_samples` advances only when a DMA buffer tagged with body audio has
physically completed. `credit_ms` accounts for both software-queued and
DMA-resident audio; the gateway must not exceed it.

After the last frame, the gateway sends:

```json
{
  "type": "audio.playback.close",
  "protocol_major": 1,
  "lease_id": "daemon-lease-uuid",
  "turn_id": "voice_daemon_turn_id",
  "stream_id": 32012
}
```

The body drains already accepted PCM before disabling I2S, then sends:

```json
{
  "type": "audio.playback.drained",
  "protocol_major": 1,
  "lease_id": "daemon-lease-uuid",
  "turn_id": "voice_daemon_turn_id",
  "stream_id": 32012
}
```

If physical drain times out or fails, the body sends failure rather than
claiming completion:

```json
{
  "type": "audio.playback.failed",
  "protocol_major": 1,
  "lease_id": "daemon-lease-uuid",
  "turn_id": "voice_daemon_turn_id",
  "stream_id": 32012,
  "reason": "physical_playback_timeout"
}
```

The body retains the route until the daemon answers with terminal
`turn.state: idle`. A transport loss or explicit abort discards the queue
immediately. Abort and failed I2S disable cannot satisfy a waiter expecting
`audio.playback.drained`; terminal outcomes are bound to stream generation.

The body rejects:

- non-mono audio;
- rates other than 24 kHz for PCM V1;
- frames other than 20 ms;
- data for a stream that is not open;
- playback while a half-duplex microphone capture is active.

## Binary media frame

One WebSocket binary message contains one complete frame:

```text
offset size field
0      1    protocol_major
1      1    kind
2      2    flags, big endian
4      4    stream_id, big endian
8      4    sequence, big endian
12     8    monotonic_timestamp_us, big endian
20     N    payload
```

Kinds:

| Value | Payload |
|---:|---|
| `0x01` | Microphone PCM16LE, 320 samples, 640 bytes |
| `0x02` | Speaker PCM16LE, 480 samples, 960 bytes |
| `0x03` | Reserved Opus packet |
| `0x04` | Reserved bounded diagnostics |

Flags:

| Bit | Meaning |
|---:|---|
| 0 | Discontinuity before this frame |

All other flag bits are reserved and must be zero. After the first frame,
receivers require contiguous sequence numbers. The first retained frame after a
gap is accepted only when its discontinuity flag is set. Stream completion uses
the correlated `audio.capture.close` and `audio.playback.close` control messages,
not a binary-frame flag.

Sequence numbers are per stream. PCM sample bytes are little-endian. Media
timestamps are monotonic and are not wall-clock timestamps.

## Face state

The gateway sends desired semantic state:

```json
{
  "type": "face.set",
  "protocol_major": 1,
  "lease_id": "...",
  "turn_id": "...",
  "state_version": 18,
  "expression": "thinking",
  "caption": "Working on that...",
  "accent_rgb": 13007708,
  "ttl_ms": 5000
}
```

The hub includes the current `lease_id` and `turn_id`. Bodies ignore correlated
face updates for stale routes. Both fields may be omitted only by legacy
minor-1 controllers; sending just one is malformed.

Supported expressions:

```text
booting
unprovisioned
connecting
idle
attentive
listening
thinking
speaking
happy
concerned
muted
offline
updating
error
sleeping
```

The body renders and animates these locally. The gateway never sends arbitrary
pixel or LVGL commands.

## Face theme

The gateway selects one of the built-in face themes. Themes change the
decorative shell, eye style, mouth geometry, caption placement, and default
accent; expressions, mouth motion, captions, and the microphone indicator keep
working identically on every theme.

```json
{
  "type": "theme.set",
  "protocol_major": 1,
  "state_version": 4,
  "theme_id": "kettle-soft-bowl"
}
```

Supported theme identifiers:

```text
warm-aperture
warm-lintel
warm-ledge
kettle-tall-bright
kettle-soft-bowl
solar-wide-awake
solar-crown
```

`state_version` must be a positive integer. Bodies apply a theme only when
`state_version` is strictly greater than the last applied version. A repeat of
the current version with the same `theme_id` is accepted as an idempotent
no-op; the same version with a different `theme_id` is ignored. Any other
`theme_id` value, including `default`, is malformed.

Bodies persist the selected theme identifier as the local pre-session default.
The version gate itself resets when a new session becomes ready, so the gateway
resends `theme.set` after every `welcome`; a positive session version always
wins over the restored local selection.

## Turn state

Gateway:

```json
{
  "type": "turn.state",
  "protocol_major": 1,
  "lease_id": "daemon-lease-uuid",
  "turn_id": "voice-turn-id",
  "state": "thinking"
}
```

States:

```text
idle
requesting_capture
listening
waiting_for_runtime
thinking
speaking
interrupted
aborted
```

Invalid state transitions are rejected locally and logged as protocol errors.

## Controls and telemetry

Button event:

```json
{
  "type": "button.event",
  "protocol_major": 1,
  "boot_id": "7f81d394f3f945de873c18a0ea40db93",
  "event_id": 42,
  "button": "volume_up",
  "action": "pressed",
  "monotonic_us": 87412200
}
```

Battery:

```json
{
  "type": "battery.state",
  "protocol_major": 1,
  "raw_adc": 2280,
  "percent": 67,
  "charging": false
}
```

The hardware exposes charge-active, not reliable USB/external-power presence.
`battery.state` therefore has no `external_power` field.

Health:

```json
{
  "type": "health.report",
  "protocol_major": 1,
  "uptime_s": 3600,
  "wifi_rssi_dbm": -54,
  "free_internal_heap": 131072,
  "minimum_internal_heap": 104448,
  "free_psram": 7340032,
  "audio_rx_overruns": 0,
  "audio_tx_underruns": 0,
  "lifecycle": "online",
  "turn": "idle",
  "microphone_muted": false,
  "app_version": "0.2.0",
  "build_number": 2,
  "running_slot": 0,
  "ota_pending_verification": false,
  "reset_reason": 1,
  "transport_generation": 3,
  "core_queue_depth": 0,
  "core_queue_capacity": 16,
  "core_stack_free_words": 900,
  "display_stack_free_words": 800,
  "display_flush_submitted": 100,
  "display_flush_errors": 0,
  "display_flush_timeouts": 0,
  "display_flush_last_us": 2500,
  "display_flush_max_us": 4100,
  "playback_sequence_gaps": 0,
  "temperature_millicelsius": 41250,
  "cpu_frequency_mhz": 80,
  "display_frame_rate": 5,
  "backlight_percent": 35
}
```

Volume command:

```json
{
  "type": "volume.set",
  "protocol_major": 1,
  "percent": 70
}
```

## Ownership and claim revocation

Credential rotation through the Security 2 `bobe-claim` endpoint requires a
controller epoch strictly greater than the active claim. The new claim is
written to the inactive slot and atomically activated before the previous slot
is retired. If cleanup is interrupted, the
inactive slot is retired idempotently when provisioning next starts.

An authenticated owner can revoke the active claim only while the body is
online and idle:

```json
{
  "type": "claim.revoke",
  "protocol_major": 1
}
```

The message has exactly those two fields. The body stops BodyLink, invalidates
the active claim marker before deleting credential remnants, displays the
unprovisioned face, and opens a bounded Security 2 provisioning window. A claim
marked ready without a valid active-slot marker is inconsistent and is never
accepted as provisioned.

## Signed firmware updates

The authenticated gateway may offer a newer WB-12 image. `manifest` is the exact
canonical JSON string signed with ECDSA P-256/SHA-256; `signature` is the base64
DER signature. Firmware verifies the signature before using the URL or any
manifest field.

```json
{
  "type": "update.offer",
  "protocol_major": 1,
  "manifest": "{\"schema\":1,\"device_model\":\"wonderboy-wb12\",\"version\":\"0.2.0\",\"build_number\":2,\"image_size\":2000000,\"image_sha256\":\"64-lowercase-hex-digits\",\"url\":\"https://updates.example/bobe.bin\",\"key_id\":\"prod-2026-01\",\"not_before\":1700000000,\"expires_at\":1800000000}",
  "signature": "base64-DER-ECDSA-signature"
}
```

Every field is mandatory. Unknown or duplicate fields, non-integer numeric
forms, wrong hardware, non-HTTPS URLs, images over 5 MiB, and build numbers not
strictly greater than the running `CONFIG_BOBE_BUILD_NUMBER` are rejected.
Trusted wall time enforces the half-open validity interval
`[not_before, expires_at)`.

The body reports acceptance, rejection, failure, and terminal reboot state:

```json
{
  "type": "update.state",
  "protocol_major": 1,
  "phase": "failed",
  "progress_percent": 0,
  "reason": "low_battery"
}
```

An offer is accepted only while online and idle, and when the last battery
sample is at least 50 percent or charging. The image is downloaded using the
claimed server CA, streamed to the inactive OTA slot, checked against the exact
size and SHA-256 digest, validated as an ESP image, and selected for boot only
after every check succeeds. A new image must restore an authenticated BodyLink
session within ten minutes; otherwise firmware marks it invalid and reboots to
the previous slot.

## Queue and failure rules

- Control has priority over audio.
- Playback credit is measured in milliseconds.
- V1 body playback is bounded to 300 ms.
- Stale unsent media is dropped, never accumulated.
- Button events are identified by `{boot_id,event_id}`.
- Reconnect discards every audio stream.
- Control input is capped at 4096 bytes before cJSON parsing. Malformed,
  duplicate-key, imprecise-number, and route-invalid messages fail the session;
  firmware minor 1 ignores unknown message types for forward compatibility.
- Wi-Fi or gateway loss produces an honest connecting/reconnecting face.
- No ambient audio is written to flash or uploaded after reconnect.

## Speech placement

The primary path is:

```text
WB-12 PCM16 16 kHz
        |
Rust BodyLink gateway
        |
readiness-qualified daemon STT
        |
authoritative BoBe turn
        |
daemon TTS and gateway Opus decoding
        |
BodyLink PCM16 24 kHz
```

When daemon STT is unavailable, the gateway sends the same daemon-owned route
and microphone frames to the scoped loopback Swift adapter for transcription.
That fallback returns only correlated transcript events; it does not own turn
admission, routing, persistence, tools, or model authority.

The broad daemon API is loopback-only by default. BodyLink is a separate
WSS surface with CA validation plus exact enrolled-leaf pinning. The Swift
adapter connects to a distinct bearer-authenticated loopback socket only when
the fallback is needed. Rust owns turn admission, endpoint-only stream
delivery, canonical conversation resynchronization, captions, playback credit,
and lease cleanup.
