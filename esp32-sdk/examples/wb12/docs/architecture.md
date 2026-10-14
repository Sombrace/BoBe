# BoBe WB-12 body architecture

The design of the BoBe body firmware on the WB-12. The shared runtime is in the
SDK's `components/`; the WB-12 implementations are in this example's
`components/bobe_board_port` ([porting guide](../../../docs/porting.md)).

Target: WonderBoy WB-12, ESP32-S3R8, 16 MiB flash, 8 MiB octal PSRAM, ESP-IDF
6.0.2.

## 1. Goal

Make the WB-12 an expressive body for BoBe without moving intelligence, memory,
credentials or tool authority onto the toy: the body is a surface of the one
BoBe described in [`PRODUCT.md`](../../../../PRODUCT.md), not another agent.

The device owns deterministic real-time behavior: the locally rendered face and
device state; microphone capture and speaker playback; buttons, volume,
battery, charge state and safe shutdown; Wi-Fi, pairing, reconnect, health and
OTA rollback confirmation; bounded offline behavior; and optional local VAD,
audio front end, pre-roll and WakeNet.

The BoBe runtime owns conversation state and memory, speech recognition and
endpointing, model sessions, tools, permissions, goals and proactive policy,
reply text and speech synthesis, and routing across bodies.

Hardware facts come from this example's [README](../README.md); runtime
placement and ownership from BoBe's [architecture](../../../../docs/architecture.md);
voice, embodiment and privacy rules from
[physical BoBe](../../../../docs/physical-bobe.md).

## 2. Hardware contract

| Resource | Contract |
|---|---|
| MCU | ESP32-S3 QFN56 rev 0.2, dual Xtensa LX7 at 240 MHz |
| PSRAM | 8 MiB AP Memory octal, 80 MHz |
| Flash | 16 MiB Boya B.25Q128AS |
| USB | Native USB Serial/JTAG on GPIO19/20 |

Internal SRAM is the scarce resource. UI objects, assets, protocol scratch,
non-DMA queues and ESP-SR and transport worker stacks live in PSRAM; I2S DMA
descriptors, short DMA buffers, ISR data and core audio tasks stay internal.

| Display property | Value |
|---|---|
| Controller | ST7789, 240 x 280 active, 280 x 240 logical landscape, RGB565 |
| Bus | SPI3, mode 3, 40 MHz pixel clock |
| Pins | SCLK 9, MOSI 10, CS 14, D/C 8, reset 18, backlight 13 (25 kHz LEDC PWM) |
| Transform | X offset 20, swap axes, mirror X, invert colors |

| Audio path | Controller | Rate | Slot | Pins |
|---|---|---:|---|---|
| Microphone | I2S1 RX | 16 kHz | Mono left, 32-bit | WS 4, BCLK 5, DIN 6 |
| Speaker | I2S0 TX | 24 kHz | PCM16 in mono-left 32-bit slots | DOUT 7, BCLK 15, WS 16 |

Application and speaker DMA use signed PCM16LE; I2S places each sample in a
32-bit Philips slot, keeping the recovered wire clock at half the DMA traffic.

| Function | GPIO | Behavior |
|---|---:|---|
| Action / boot | 0 | Active-low; ROM strap |
| Volume down | 39 | Active-low |
| Volume up | 40 | Active-low |
| Power hold | 21 | Driven high before any allocation, network or UI work; low requests shutdown |
| Charge status | 38 | Active-low |
| Battery ADC | 17 | ADC2 channel 6, 12 dB attenuation |

## 3. Firmware stack

Exact pins:

| Layer | Selection | Version | Reason |
|---|---|---:|---|
| SDK | ESP-IDF | 6.0.2 | Tested WB-12 baseline |
| UI | LVGL | 9.5.0 | Native embedded UI and animation |
| UI integration | `espressif/esp_lvgl_port` | 2.8.0~1 | Display, task and tick integration |
| Transport | `espressif/esp_websocket_client` | 1.7.0 | Supported WSS client with bounded events |
| Provisioning | `espressif/network_provisioning` | 1.2.4 | ESP-IDF provisioning |
| Buttons | Local fixed-state debouncer | project code | Three known GPIOs, bounded polling, no dynamic objects |
| Discovery | `espressif/mdns` | 1.11.3 | Local hub discovery |
| Speech front end | `espressif/esp-sr` | 2.4.6 | Optional AFE, VAD, pre-roll and custom WakeNet |

## 4. Graphics

The face is procedural LVGL state, not a remotely rendered UI or a sequence of
full-screen images. The runtime sends semantic state in
[`face.set`](bodylink-v1.md#face-state); the body animates it locally: vector
eyes with blink, gaze, squash and breathing; a mouth driven by the speaking
envelope or a state animation; solid RGB565 backgrounds that avoid gradient
banding; an optional caption; and status text plus a microphone overlay. The
microphone indicator cannot be overridden or hidden by remote state. The newest
`state_version` wins, and expired remote state falls back to a local state.

### Local notices

Physical controls get visible acknowledgement even when the controller is slow
or absent. A notice is a short local string with a time-to-live, carried by
`bobe_notice_t` and composed by the pure `bobe_notice` module:

| Kind | Trigger | TTL |
|---|---|---|
| `volume` | A volume button changed the level | 1.5 s |
| `silent` | Volume reached zero | 1.5 s |
| `hint` | Lifecycle or mute change: which control helps now | 4 s |
| `update` | A pending OTA image was confirmed at session start | 5 s |

Notices never write `bobe_face_state_t`, `state_version` or the theme, so the
controller keeps sole ownership of expression, accent and theme. They take the
caption line only while showing, then the controller's caption returns; the
status pill and expression stay untouched. Microphone mute is durable state
shown as `BOBE_FACE_MUTED` with a "Microphone muted" caption, not a notice; muting
queues a `hint` because the mute face does not say how to undo it. `silent` is
worded so it cannot be mistaken for microphone mute.

Lifecycle, mute and OTA notices are queued in the policy context and flushed on
the next tick, keeping the fixed `BOBE_POLICY_MAX_EFFECTS` budget on crowded
boot, teardown and mute paths; volume notices are emitted inline. A newer queued
notice replaces an unflushed one. Expiry runs on the UI task against the frame
clock, and the renderer stays a pure function of its input. The face shows no
download progress: a pending image is already running, so it shows completion;
install progress goes upstream in `update.state`.

### Performance

- 280 x 240 RGB565 at 40 MHz SPI, about 5 MB/s before overhead.
- 30 FPS for active motion, lower when idle.
- One 20-line DMA buffer uses 11.2 KiB of internal SRAM; the LVGL heap, objects
  and assets use PSRAM.
- Only dirty regions flush.
- One LVGL task makes every LVGL call; network and audio tasks only publish
  desired state, and face commands coalesce.

Procedural eyes and mouth cost a small fraction of a full-screen flush per frame,
giving better quality and lower power than decoding GIFs.

### Expression and status rail

[BodyLink V1](bodylink-v1.md#face-state) lists the expressions. A fixed,
theme-colored rail outside the face shows three Wi-Fi bars at top left and a
battery capsule at top right, red at 15% and green while charging, from local
hardware state only. Short transient captions (`Hold O: talk`, `Hold -: mic`,
`Hold +/-: setup`) teach the controls without permanent text.

### Power

There is no automatic light sleep:

- active turns, capture, notices and recent interaction use 75% backlight and
  30 FPS;
- after 60 seconds online and idle, 35% and 5 FPS; any button, wake event, notice
  or turn restores full output;
- setup, reconnect and low battery stay readable at 50% and 10 FPS;
- dynamic frequency scaling spans 80–240 MHz;
- Wi-Fi stays at full power during BodyLink sessions, because minimum modem sleep
  caused speaker underruns at moderate RSSI;
- light sleep, tickless idle, PSRAM clock changes and lower tick rates stay off
  to protect USB, audio and DMA timing.

The backlight PWM runs from the 40 MHz crystal at 25 kHz and 10 bits, so CPU
frequency changes do not alter brightness.

## 5. Audio

| Direction | Format | Frame |
|---|---|---|
| Device to BoBe | PCM16LE mono 16 kHz | 20 ms, 320 samples, 640 bytes |
| BoBe to device | PCM16LE mono 24 kHz | 20 ms, 480 samples, 960 bytes |

PCM matches the hardware rates exactly, removes codec and resampler ambiguity,
keeps captures inspectable, and costs about 640 kbit/s full duplex before TLS,
well within 2.4 GHz Wi-Fi.

### Interaction

V1 is push-to-talk and honest half duplex:

1. Pressing the action button requests capture.
2. The device enters `listening` and streams bounded microphone PCM.
3. Release ends the capture.
4. The BoBe gateway recognizes speech and submits the transcript to the runtime.
5. Playback starts only after an explicit `audio.playback.open`.
6. The device reports played samples and underruns.

GPIO0 is hold-to-talk while online and its long press does not shut down;
shutdown needs volume-down and GPIO0 together. The rear button is hardware
reset and not part of the button protocol. The board has one microphone and no
verified playback reference, so the firmware claims neither AEC nor reliable
barge-in.

### Optional front end and wake word

```text
I2S mic -> ESP-SR AFE (NS -> AGC -> VAD -> optional WakeNet)
        -> 500 ms bounded pre-roll -> BodyLink audio stream
```

The factory `nihaoxiaozhi` model is not a BoBe wake model; "Hey BoBe" needs a
licensed or custom WakeNet or microWakeWord model with measured false-accept and
false-reject rates. AEC stays off without a calibrated playback reference.

### Bounded media

- I2S DMA callbacks never touch the network.
- Capture and playback queues are measured in milliseconds; initial playback
  credit is 300 ms.
- Audio past its deadline is dropped with a discontinuity report, and reconnect
  discards all old audio.
- Underrun produces silence and telemetry, never stale replay.
- Capture overrun aborts the stream rather than corrupting it.

## 6. BodyLink V1

[BodyLink V1](bodylink-v1.md) is the wire contract; this section records its
design decisions.

One persistent WebSocket over authenticated TLS: BoBe already serves Axum
WebSockets, ESP-IDF maintains a client, it works for Mac-gateway, self-hosted
and hosted runtimes, its framing, close and ping behavior are standard, its
overhead is negligible beside audio, and one connection is easier to bound than
MQTT plus UDP. The semantic frames do not depend on WebSocket.

Control messages are size-limited UTF-8 JSON text frames with required fields;
unknown optional fields are ignored. Media are binary frames with one
[fixed header](bodylink-v1.md#binary-media-frame) and payload, whose length is
the message length; PCM bytes are little-endian.

The device sends `hello`; the runtime replies `welcome` with the body session
ID, daemon-owned connection generation, controller epoch and heartbeat interval
([messages](bodylink-v1.md#identity-and-versioning)). Firmware advertises one
minor: 1 for a voice-only body such as the WB-12, 2 for bodies with text,
actions or sensors. The gateway accepts it when supported and inside the
endpoint's enrolled range, and echoes it; a major mismatch or unsupported minor
is rejected.

Control messages:

- session: `hello`, `welcome`, `ping`, `pong`, `error`, `health.report`;
- conversation and media: `capture.request`, `capture.cancel`,
  `capture.granted`, `capture.denied`, `audio.capture.open`,
  `audio.capture.close`, `audio.playback.open`, `audio.playback.ready`,
  `audio.playback.ack`, `audio.playback.close`, `audio.playback.drained`,
  `audio.playback.failed`, `turn.state`;
- body and lifecycle: `face.set`, `theme.set`, `button.event`, `volume.set`,
  `battery.state`, `claim.revoke`, `update.offer`, `update.state`.

Capture requests carry a device request ID; granted commands then carry the
daemon's lease and turn IDs. Control outranks media, sequence numbers are per
media stream, button events are identified by `{boot_id,event_id}`, audio is
never replayed after reconnect, the controller epoch fences stale runtimes, and a
dropped command is never treated as success.

Reconnect backoff is bounded and owned by the transport. Pings go every 10
seconds and three missed intervals end the session. Each boot has a new
`boot_id`, and every reconnect discards audio and repeats `hello`/`welcome`.

## 7. Firmware components

```text
app_main
  +-- bobe_board         power hold, pins, backlight, buttons, ADC, charge state
  +-- bobe_display       esp_lcd, LVGL port, procedural face, local fallback
  +-- bobe_audio         I2S RX/TX, PCM conversion, bounded queues, levels
  +-- bobe_connectivity  NVS, Wi-Fi, provisioning, SNTP, mDNS, network state
  +-- bobe_protocol      strict JSON control, binary header, limits, IDs
  +-- bobe_transport     WSS lifecycle, prioritized TX, reconnect, heartbeat
  +-- bobe_core          state machines, event routing, policy, orchestration
```

Components exchange typed events and never reach into each other's internals.

## 8. FreeRTOS execution

[Runtime schedulability](runtime-schedulability.md) lists each task's core,
priority and deadline; `bobe_task_config.h` is canonical. ESP-IDF system tasks
keep their higher Core-0 priorities (Wi-Fi 23, event loop 20, TCP/IP 18), so
LVGL cannot preempt audio and network work stays below the stack it uses.

- No socket writes from I2S or GPIO callbacks; I2S callbacks do only bounded
  internal-RAM metadata and PCM copies.
- No LVGL calls outside the LVGL task.
- No unbounded queue and no infinite wait on another subsystem.
- Safety-critical queue overflow is surfaced and tears down the session.
- Allocation after a steady connection is exceptional.

Speaker startup scales PCM before the ISR, preloads every READY DMA descriptor,
then enables I2S. Credit counts software and DMA-resident frames. Starvation
plays silence until a bounded refill threshold rather than replaying stale DMA.
Drain, abort and failure are distinct, generation-bound outcomes.

## 9. State machines

```text
booting -> unprovisioned -> joining_wifi -> discovering_hub -> connecting -> online
online -> reconnecting -> online
online -> updating -> rebooting
any    -> low_battery -> shutting_down
any    -> fatal_error
```

The body never shows "listening" without a capture lease or "online" without an
authenticated session.

```text
idle -> requesting_capture -> listening -> waiting_for_runtime -> thinking -> speaking -> idle
speaking -> interrupted -> listening
any      -> aborted -> idle
```

There is one audible stream; earcons, status sounds and proactive audio share the
speech playback arbiter.

## 10. Wi-Fi, discovery and provisioning

Development builds take Wi-Fi from an uncommitted `sdkconfig`, stored state, or
Security 2 SoftAP provisioning. Per-device Security 2 material is generated under
`private/provisioning`; the SRP password stays in its private manifest and
firmware gets only the salt and verifier. A separate random WPA2 password
protects the temporary SoftAP. No password is compiled into tracked source.

The firmware runs a Security 2 SoftAP service and a private `bobe-claim` payload
holding the WSS endpoint, trust-key ID, controller epoch, private CA and
per-device client certificate. mbedTLS parses the CA, certificate and key and
verifies the pair before any claim state is written. Holding both volume buttons
opens a time-bounded recovery window, and a replacement must carry a newer
controller epoch. Claim and TLS data are written to the inactive slot before one
active-slot marker flips, so a failed replacement leaves the previous claim
active; completion hot-reloads the identity and cancellation restores the
previous transport. Once provisioned, the body uses a configured endpoint URI or
discovers `_bobe-body-hub._tcp` over mDNS on a trusted LAN, prefiltering by
trust-key ID before TLS authenticates the server against the stored CA.
Discovery is not authentication.

## 11. Storage and partitions

BoBe uses its own partition table, not the
[factory layout](../README.md#flash-layout):

| Partition | Offset | Size | Purpose |
|---|---:|---:|---|
| `nvs` | `0x9000` | 24 KiB | Wi-Fi, pairing, settings, counters |
| `otadata` | `0xF000` | 8 KiB | OTA selection and rollback |
| `phy_init` | `0x11000` | 4 KiB | PHY data |
| `nvs_keys` | `0x12000` | 4 KiB | NVS encryption keys; encryption is off in development |
| `ota_0` | `0x20000` | 5 MiB | Application slot A |
| `ota_1` | `0x520000` | 5 MiB | Application slot B |
| `model` | `0xA20000` | 2 MiB | ESP-SR model pack |
| `assets` | `0xC20000` | 3.75 MiB | Factory or future UI assets; not mounted by the procedural UI |
| `coredump` | `0xFE0000` | 128 KiB | Bounded crash dump |

```text
board          optional factory UUID
bobe_identity  generated stable device UUID
bobe_net       endpoint, trust-key ID, controller epoch, active-slot marker
bobe_tls       development CA, client certificate and private key
bobe_clock     last authenticated wall-clock floor
```

Raw audio is never written to flash. SNTP sets the live clock but is not trusted
for persistence: a bounded clock floor is written only after an authenticated
session, and floors outside the supported window are ignored. Without flash
encryption, the private key in `bobe_tls` is recoverable with physical access,
which is not acceptable for production.

## 12. OTA and rollback

An authenticated gateway sends a canonical P-256-signed `update.offer`. Firmware
checks the hardware model, monotonic build number, trusted-time window, HTTPS
URL, size and SHA-256 while streaming to the inactive slot, and selects it only
after ESP image validation. The new image must reach an authenticated session
within ten minutes of boot to be confirmed; otherwise it is marked invalid and
the device reboots into the previous slot. Updates are refused while busy, below
50% battery when not charging, or before trusted time. `update.state` reports
acceptance, failure, progress and reboot. The gateway loads only a pre-signed
envelope; signing keys stay offline.

Production needs signed images, Secure Boot v2, flash encryption and
anti-rollback. These irreversible controls are not burned in development;
[release security](release-security.md) covers their enrollment. Factory
restoration is separate from application rollback.

## 13. Security

An unmodified WB-12 has Secure Boot and flash encryption off, and its factory
HMAC key is read-protected and must not be repurposed.

Development rules:

- validate the WSS server certificate;
- reject plaintext non-loopback endpoints unless a development configuration
  allows them;
- never log Wi-Fi passwords, keys, certificates, tokens or audio;
- keep provider, MCP and runtime credentials off the device;
- keep the recovery provisioning window time-bounded, and require physical
  presence in a production policy;
- bound every remote payload before allocation;
- show microphone and connectivity state unmistakably.

Production adds per-device certificates and revocation, mutual TLS, encrypted
NVS, Secure Boot v2 and flash encryption, signed A/B OTA with anti-rollback,
credential rotation and ownership transfer, and a hardware microphone cutoff on
future hardware.

A lone long press of volume-down toggles microphone mute: policy stops capture and
wake monitoring, rejects new capture requests, ignores remote face overrides, and
shows the muted face with a hint for undoing it. Mute persists as `mic_muted`
across boots. It is a software control, not a hardware cutoff.

Credential rotation writes a strictly newer controller epoch to the inactive slot,
activates it atomically and retires the old slot; interrupted retirement resumes
idempotently at the next provisioning. An authenticated `claim.revoke` clears the
claim and both slots, stops BodyLink and returns to bounded Security 2
provisioning. A ready claim without a valid active-slot marker fails closed.

## 14. Offline behavior

Without BoBe or Wi-Fi the body stops remote capture and playback, shows an
offline face, keeps volume, theme, mute, battery safety, shutdown and local
animation, and reconnects with bounded backoff. It never buffers audio for later
upload or falls back to another assistant.

## 15. Health

`health.report` carries bounded numeric metadata
([fields](bodylink-v1.md#controls-and-telemetry)) and never raw audio,
transcripts by default, Wi-Fi credentials, certificates, keys, provider
credentials or memory. Battery and charge go in `battery.state`. BoBe's
[architecture](../../../../docs/architecture.md) covers the runtime side and the
[speech placement](bodylink-v1.md#speech-placement) section shows how body audio
reaches recognition and replies return as PCM.

## 16. Validation

Lifecycle decisions pass through a pure `bobe_policy` reducer: typed input plus
explicit time and random values produce a bounded effect batch, executed by the
ESP adapter. Host tests run the same policy as the device. The widget renderer is
shared by the LCD adapter and an LVGL 9.5 host test that freezes phase, tick and
scroll and compares 280 x 240 RGB565 hashes; a correct logical render does not
prove the write-only panel.

The device runs only the normal product image and validation observes it from
outside, so production logs state what it does: an identity line
(`device=... boot=... firmware=... project=... elf=...`), periodic health lines and
one line per debounced button edge
(`physical button event_id=... button=... action=... monotonic_us=...`), which
every compile-only profile gate checks for.
[`hil/README.md`](../hil/README.md) and [`hil-toolchain.md`](hil-toolchain.md)
describe the fixtures, tool versions and safety boundary.

Build and test paths only compile: `tools/idf.sh build`, the host tests and
`tools/check.sh` never flash or actuate hardware. Flashing is the explicit
`tools/bobe_hil.sh device flash`, or `tools/flash_app_verified.sh` for the fixed
`ota_0` path ([tools](../tools/README.md)). Keep owner backups intact. A build
does not qualify battery reporting, power and thermal behavior, multi-day
reliability, OTA recovery, acoustics, production enrollment or BLE; follow the
[SDK validation guidance](../../../docs/validation.md) before making hardware
claims.

References: [ESP-IDF 6.0.2](https://docs.espressif.com/projects/esp-idf/en/v6.0.2/esp32s3/),
[ESP-SR](https://github.com/espressif/esp-sr), [LVGL 9.5](https://github.com/lvgl/lvgl/tree/v9.5.0),
[RFC 6455](https://www.rfc-editor.org/rfc/rfc6455), [RFC 8446](https://www.rfc-editor.org/rfc/rfc8446).
