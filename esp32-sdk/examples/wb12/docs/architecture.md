# BoBe WB-12 Body Architecture

This document describes the design of the BoBe body firmware on the WB-12. The
shared runtime lives in the SDK's root `components/`, and the WB-12
implementations live in this example's `components/bobe_board_port`. See the
[SDK porting guide](../../../docs/porting.md) for that boundary.

Target: WonderBoy WB-12, ESP32-S3R8, 16 MiB flash, 8 MiB octal PSRAM
Firmware baseline: ESP-IDF 6.0.2

## 1. Goal

Turn the WB-12 into an expressive physical body for BoBe without moving
BoBe's intelligence, memory, credentials, or tool authority onto the toy.

The device owns deterministic real-time behavior:

- a locally rendered face and device state;
- microphone capture and speaker playback;
- buttons, volume, battery, charge state, and safe shutdown;
- Wi-Fi, pairing, reconnect, health, and OTA rollback confirmation;
- bounded offline behavior;
- optional local VAD, audio front end, pre-roll, and WakeNet.

The authoritative BoBe runtime owns:

- conversation state and long-term memory;
- speech recognition and language endpointing;
- language-model and Copilot sessions;
- tools, permissions, goals, and proactive policy;
- response text and speech synthesis;
- routing across bodies.

This preserves the BoBe product invariant in BoBe's `PRODUCT.md`:

> one person, one BoBe, one durable agent identity, many possible surfaces and
> model providers

The WB-12 is a body, not another agent.

## 2. Evidence and authority

This architecture reconciles three sources:

1. This example's [`README.md`](../README.md) is authoritative for WB-12
   hardware.
2. BoBe's [`docs/architecture.md`](../../../../docs/architecture.md) is
   authoritative for runtime placement, identity, and multi-body ownership.
3. BoBe's [`docs/physical-bobe.md`](../../../../docs/physical-bobe.md) supplies
   the product's voice, embodiment, and privacy rules.

## 3. Exact hardware contract

### 3.1 Processor and memory

| Resource | Contract |
|---|---|
| MCU | ESP32-S3 QFN56 revision 0.2 |
| CPU | Dual Xtensa LX7, 240 MHz |
| PSRAM | 8 MiB AP Memory octal PSRAM, 80 MHz |
| Flash | 16 MiB Boya B.25Q128AS |
| USB | Native USB Serial/JTAG on GPIO19/20 |

Internal SRAM is the scarce resource. Large UI objects, assets, protocol
scratch storage, non-DMA queues, and ESP-SR/transport worker stacks use PSRAM.
I2S DMA descriptors, short DMA buffers, ISR data, and the core audio tasks
remain internal.

### 3.2 Display

| Property | Value |
|---|---|
| Controller | ST7789 |
| Active pixels | 240 x 280 |
| Logical resolution | 280 x 240 landscape |
| Pixel format | RGB565 |
| SPI host | SPI3 |
| SPI mode | 3 |
| Pixel clock | 40 MHz |
| SCLK | GPIO9 |
| MOSI | GPIO10 |
| CS | GPIO14 |
| D/C | GPIO8 |
| Reset | GPIO18 |
| Backlight | GPIO13, 25 kHz LEDC PWM |
| Transform | X offset 20, swap axes, mirror X, invert colors |

### 3.3 Audio

| Path | Controller | Rate | Slot | Pins |
|---|---|---:|---|---|
| Microphone | I2S1 RX | 16 kHz | Mono left, 32-bit | WS 4, BCLK 5, DIN 6 |
| Speaker | I2S0 TX | 24 kHz | PCM16 in mono-left 32-bit slots | DOUT 7, BCLK 15, WS 16 |

The application and speaker DMA formats are signed PCM16LE. The I2S peripheral
places each 16-bit sample in a 32-bit Philips slot, preserving the recovered
wire clock while halving DMA traffic.

### 3.4 Controls and power

| Function | GPIO | Behavior |
|---|---:|---|
| Action / boot | 0 | Active-low; ROM strap |
| Volume down | 39 | Active-low |
| Volume up | 40 | Active-low |
| Power hold | 21 | Drive high immediately; low requests shutdown |
| Charge status | 38 | Active-low |
| Battery ADC | 17 | ADC2 channel 6, 12 dB attenuation |

The power-hold GPIO is initialized before any allocation, network, or UI work.

## 4. Firmware stack

Versions are exact pins.

| Layer | Selection | Version | Reason |
|---|---|---:|---|
| SDK | ESP-IDF | 6.0.2 | Tested WB-12 baseline |
| UI | LVGL | 9.5.0 | Current native embedded UI and animation engine |
| UI integration | `espressif/esp_lvgl_port` | 2.8.0~1 | Espressif display/task/tick integration |
| Transport | `espressif/esp_websocket_client` | 1.7.0 | Supported WSS client with bounded event integration |
| Provisioning | `espressif/network_provisioning` | 1.2.4 | Current ESP-IDF provisioning component |
| Buttons | Local fixed-state debouncer | project code | Three known GPIOs; bounded polling with no dynamic objects |
| Discovery | `espressif/mdns` | 1.11.3 | Local BoBe hub discovery |
| Speech front end | `espressif/esp-sr` | 2.4.6 | Optional AFE, VAD, pre-roll, and custom WakeNet profile |

## 5. Graphics architecture

### 5.1 Rendering strategy

BoBe's face is procedural LVGL state, not a remotely rendered UI and not a
sequence of hard-coded full-screen images.

The runtime sends semantic desired state in
[`face.set`](bodylink-v1.md#face-state). The body maps that to local animation:

- eyes: rounded vector objects with blink, gaze, squash, and breathing motion;
- mouth: local speaking envelope or deterministic state animation;
- background: solid RGB565 colors to avoid visible gradient quantization;
- caption/status: a small optional layer;
- status text for setup/connectivity/activity plus a local microphone overlay.

The local microphone-active indicator is a non-overridable overlay. Remote face
state may change expression but cannot hide capture state.

Newest `state_version` wins. Expired remote state falls back to a local state.

### 5.1.1 Local notices

Physical controls need visible acknowledgement even when the controller is
slow, busy, or absent. A notice is a short local string with a time-to-live,
carried by `bobe_notice_t` and composed by the pure `bobe_notice` module:

| Kind | Trigger | TTL |
|---|---|---|
| `volume` | volume button changed the level | 1.5 s |
| `silent` | volume reached zero, so output is silent | 1.5 s |
| `hint` | lifecycle or mute change: which control is useful now | 4 s |
| `update` | a pending OTA image was confirmed at session start | 5 s |

Notices do **not** touch authoritative state:

- they never write `bobe_face_state_t`, `state_version`, or the theme, so the
  controller keeps sole ownership of expression, accent, and theme;
- they occupy the caption line only while showing, then the controller's
  caption returns verbatim;
- the status pill and expression are untouched, so `Listening`/`Speaking`
  remain readable while a volume notice is up.

Microphone mute is **not** a notice. That state is durable rather than
transient, and the state machine already surfaces it as `BOBE_FACE_MUTED` with
a "Microphone muted" caption; a competing notice would flicker the caption and
restore the same message. The `silent` notice is about the speaker reaching
zero volume and is worded to avoid any confusion with microphone mute. Muting
does queue a `hint`, because the mute face does not itself say how to undo it
and push-to-talk does nothing while muted.

Notices from lifecycle changes, mute toggles, and OTA confirmation are queued
in the policy context and flushed by the next time tick rather than appended to
the current batch. That keeps the fixed `BOBE_POLICY_MAX_EFFECTS` budget intact
on the crowded boot, teardown, and mute-reset paths. Volume notices are emitted
inline because the child is holding the button and that batch is short. A newer
queued notice replaces an unflushed one; only the most recent action is worth
showing.

Expiry runs on the UI task, which compares the frame clock against the TTL. The
renderer itself stays a pure function of its input and schedules nothing.

There is no OTA download progress bar on the face. When the body observes a
pending image it is already running that image, so the honest presentation is a
completion confirmation. Percentage progress during an install is reported
upstream over BodyLink via `update.state`, where the controller can narrate it.

### 5.2 Performance budget

- Logical display: 280 x 240 RGB565 after the hardware axis swap.
- SPI: 40 MHz, theoretical 5 MB/s before protocol overhead.
- UI target: 30 FPS for active face motion; lower when idle.
- One 20-line RGB565 DMA buffer requires 11.2 KiB of internal SRAM.
- LVGL heap, object metadata, and optional assets use PSRAM.
- Only dirty regions flush; unchanged backgrounds are not resent.
- One LVGL owner task performs every LVGL call.
- Network and audio tasks publish desired state; they never call LVGL.
- Face commands coalesce rather than queueing every obsolete animation step.

Procedural eyes and mouth should stay below a small fraction of a full-screen
flush per frame. This gives better visual quality and lower power than decoding
large GIFs continuously.

### 5.3 Expression model

The expression vocabulary is semantic;
[BodyLink V1](bodylink-v1.md#face-state) lists the expressions.

The body also reserves a fixed, theme-coloured status rail outside the face
layout: three Wi-Fi strength bars at top-left and a battery capsule at top-right.
Battery fill turns red at 15% and green while charging. The rail stays in the
same position across every face theme and is sourced only from local hardware
state. Control discovery uses short transient captions (`Hold O: talk`,
`Hold -: mic`, `Hold +/-: setup`) rather than permanent text, so the small screen
never overlaps face captions.

### 5.4 Power-aware lifecycle

The body keeps full responsiveness while active and reduces idle heat without
automatic light sleep:

- active turns, microphone capture, local notices, and recent interaction use
  75% backlight and 30 FPS;
- after 60 seconds online and idle, the backlight drops to 35% and face motion
  to 5 FPS; any button, wake event, notice, or turn restores full output
  immediately;
- setup, reconnect, and low-battery states remain readable at 50% and 10 FPS;
- CPU dynamic frequency scaling spans 80-240 MHz;
- Wi-Fi remains in full-power mode during BodyLink sessions because minimum
  modem sleep caused measurable speaker underruns at moderate RSSI;
- light sleep, tickless idle, PSRAM clock changes, and lower scheduler tick rates
  remain disabled to preserve USB, audio, and DMA timing.

The backlight PWM uses the fixed 40 MHz crystal at 25 kHz/10-bit resolution so
CPU/APB frequency changes do not alter brightness.

## 6. Audio architecture

### 6.1 V1 media profile

| Direction | Format | Frame |
|---|---|---|
| Device to BoBe | PCM16LE, mono, 16 kHz | 20 ms, 320 samples, 640 bytes |
| BoBe to device | PCM16LE, mono, 24 kHz | 20 ms, 480 samples, 960 bytes |

PCM is the LAN profile because it:

- exactly matches the recovered hardware rates;
- removes codec and resampler ambiguity from board bring-up;
- adds negligible private-LAN bandwidth;
- makes captures and failures directly inspectable.

Full-duplex payload is about 640 kbit/s before TLS overhead, well inside normal
2.4 GHz Wi-Fi capacity.

### 6.2 Interaction mode

V1 is push-to-talk and honest half-duplex:

1. Action-button press requests capture.
2. Device enters `listening` and streams bounded microphone PCM.
3. Release sends capture end.
4. The BoBe gateway performs STT and submits the final transcript to the
   authoritative runtime.
5. Playback starts only after an explicit `audio.playback.open`.
6. The device reports played samples and underruns.

GPIO0 is therefore reserved for hold-to-talk while online; its long-press
event does not shut down the body. Shutdown requires holding volume-down and
GPIO0 together. The separate rear button is hardware reset/EN and is not part
of the application button protocol.

This WB-12 has one microphone and no verified playback-reference wiring.
Firmware must not claim AEC or reliable barge-in until measured.

### 6.3 Optional AFE and wake word

The edge profile implements capability-gated ESP-SR processing:

```text
I2S mic
        |
ESP-SR AFE
NS -> AGC -> VAD -> optional WakeNet
        |
500 ms bounded pre-roll
        |
BodyLink audio stream
```

The recovered `nihaoxiaozhi` model is not a BoBe wake model. "Hey BoBe" needs a
licensed/custom WakeNet or microWakeWord model and measured false-accept and
false-reject rates. AEC remains disabled because no calibrated playback
reference exists.

### 6.4 Bounded media

- I2S DMA callbacks never touch the network.
- Capture and playback queues are measured in milliseconds.
- Initial playback credit is 300 ms.
- Audio older than its deadline is dropped with a discontinuity report.
- Reconnect discards all old audio.
- Playback underrun produces silence and telemetry, not stale replay.
- Capture overrun aborts the capture stream rather than silently corrupting it.

## 7. BodyLink V1

[BodyLink V1](bodylink-v1.md) is the wire contract. This section gives the
design decisions behind it.

### 7.1 Transport decision

V1 uses one persistent WebSocket over authenticated TLS because:

- BoBe already uses Axum WebSockets;
- ESP-IDF has a maintained client component;
- it works for Mac-gateway, self-hosted, and hosted runtime placements;
- TLS, reverse proxies, close behavior, ping/pong, and framing are standard;
- its overhead is immaterial beside audio;
- one connection is easier to bound and diagnose than MQTT plus UDP.

The semantic frame format is independent of WebSocket, so another binding can
replace WebSocket without changing the body model.

### 7.2 Control and media encodings

- Control uses UTF-8 JSON WebSocket text messages.
- Media uses WebSocket binary messages with one fixed header and one payload.
- JSON has strict size limits and required fields.
- Unknown optional JSON fields are ignored.

The wire contract specifies the
[binary media frame](bodylink-v1.md#binary-media-frame). The WebSocket message
length defines the payload length. PCM sample bytes are little-endian because
that is the negotiated media format.

### 7.3 Session negotiation

The device sends `hello` first. The runtime replies with `welcome`, including
the body session ID, daemon-owned connection generation, controller epoch, and
heartbeat interval. The wire contract shows
[both messages](bodylink-v1.md#identity-and-versioning).

The firmware advertises exactly one minor: 1 for a voice-only body with a
microphone and speaker, like the WB-12, and 2 for any other body, such as one
with text, actions or sensors. The gateway accepts that minor when it supports
it and the endpoint's enrolled protocol range includes it, and echoes it in
`welcome`. A major mismatch or an unsupported minor rejects the connection.

### 7.4 Control messages

Session:

- `hello`, `welcome`;
- `ping`, `pong`, `error`;
- `health.report`.

Conversation and media:

- `capture.request`, `capture.cancel`, `capture.granted`, `capture.denied`;
- `audio.capture.open`, `audio.capture.close`;
- `audio.playback.open`, `audio.playback.ready`,
  `audio.playback.ack`, `audio.playback.close`, `audio.playback.drained`,
  `audio.playback.failed`;
- `turn.state`.

Body and lifecycle:

- `face.set`, `theme.set`;
- `button.event`;
- `volume.set`;
- `battery.state`;
- `claim.revoke`;
- `update.offer`, `update.state`.

Capture requests use a device request ID. Every granted conversational command
then carries the daemon-issued lease and turn IDs. Button events retain
`{boot_id,event_id}` identity.

### 7.5 Ordering and backpressure

- Control is higher priority than media.
- Sequence numbers are per media stream.
- Button events use `{boot_id, event_id}`.
- Audio is never replayed after reconnect.
- A controller epoch fences stale BoBe runtimes.
- The body never treats a dropped command as success.

### 7.6 Reconnect

- Bounded reconnect backoff is transport-owned.
- WebSocket ping every 10 seconds.
- Three missed intervals terminate the session.
- A new boot creates a new `boot_id`.
- Every reconnect discards audio and performs a fresh hello/welcome handshake.

## 8. Firmware component model

```text
app_main
  |
  +-- bobe_board
  |     power hold, pins, backlight, buttons, ADC, charge state
  |
  +-- bobe_display
  |     esp_lcd, LVGL port, procedural face, local fallback state
  |
  +-- bobe_audio
  |     I2S RX/TX, PCM conversion, bounded queues, levels
  |
  +-- bobe_connectivity
  |     NVS, Wi-Fi, provisioning, SNTP, mDNS, network state
  |
  +-- bobe_protocol
  |     strict JSON control, binary header, limits, IDs
  |
  +-- bobe_transport
  |     WSS lifecycle, prioritized TX, reconnect, heartbeat
  |
  +-- bobe_core
        state machines, event routing, policy, orchestration
```

Components expose typed events. They do not reach across layers to manipulate
one another's internals.

## 9. FreeRTOS execution model

[Runtime schedulability](runtime-schedulability.md) lists each task's core,
priority and deadline. The canonical assignments live in `bobe_task_config.h`.
ESP-IDF system tasks retain their higher Core-0 priorities: Wi-Fi 23, event
loop 20 and TCP/IP 18. With these assignments, LVGL cannot preempt audio, and
network work stays below the networking stack it depends on.

Rules:

- no socket writes from I2S or GPIO callbacks;
- I2S callbacks perform only bounded internal-RAM metadata and PCM copies;
- no LVGL calls outside the LVGL owner;
- no unbounded FreeRTOS queue;
- no task waits forever on another subsystem;
- safety-critical queue overflow is surfaced and tears down the affected
  session;
- allocations after steady-state connection should be exceptional.

Speaker startup scales PCM before the ISR, preloads all READY-state DMA
descriptors, then enables I2S. Credit includes software and DMA-resident frames.
Starvation emits silence and waits for a bounded refill threshold rather than
replaying stale DMA contents. Drain, abort and failure are distinct,
generation-bound terminal outcomes.

## 10. State machines

### 10.1 Device lifecycle

```text
booting
  -> unprovisioned
  -> joining_wifi
  -> discovering_hub
  -> connecting
  -> online

online -> reconnecting -> online
online -> updating -> rebooting
any    -> low_battery -> shutting_down
any    -> fatal_error
```

Visible state must match reality. The body never shows "listening" when no
capture lease exists and never shows "online" without an authenticated session.

### 10.2 Turn lifecycle

```text
idle
  -> requesting_capture
  -> listening
  -> waiting_for_runtime
  -> thinking
  -> speaking
  -> idle

speaking -> interrupted -> listening
any      -> aborted -> idle
```

Only one local audible stream exists. Any earcon, status sound, or proactive
audio must share the same playback arbiter as speech.

## 11. Wi-Fi, discovery, and provisioning

### 11.1 Development flow

The development build supports credentials supplied through a non-committed
`sdkconfig`, stored Wi-Fi state, or Security-2 SoftAP provisioning. Per-device
Security-2 material is generated under `private/provisioning`; the SRP password
stays in its private manifest while firmware receives only salt/verifier. A
separate random WPA2 password protects the temporary provisioning SoftAP.

No password is compiled into tracked source.

### 11.2 Claim provisioning

The firmware implements a Security 2 SoftAP service and a private
`bobe-claim` payload that stores the endpoint and per-device credentials. It
implements dual-slot ownership transfer with controller-epoch rollback
protection, authenticated revocation, interrupted-retirement cleanup, and
strict client certificate/key matching.

The `bobe-claim` endpoint stores WSS endpoint, trust-key ID,
controller epoch, private CA, and per-device client certificate supplied by
the development provisioner. It parses the CA, certificate, and private key
with mbedTLS and verifies the client certificate/key pair before writing any
claim state. Holding both volume buttons opens a time-bounded Security-2
recovery window; an authenticated replacement must carry a newer controller
epoch. Claim and TLS data are committed to an inactive slot before one
active-slot marker is flipped; a failed replacement leaves the previous claim
active. Completion hot-reloads the new identity, while cancellation or startup
failure restores the prior configured transport. After provisioning, the body
can:

- use a configured endpoint URI;
- discover `_bobe-body-hub._tcp` over mDNS on a trusted LAN;
- prefilter candidates by the claimed trust-key ID before TLS authenticates the
  server against the stored private CA.

Discovery is not authentication.

## 12. Storage and partitions

The BoBe firmware uses its own partition table, not the
[factory layout](../README.md#flash-layout).

| Partition | Offset | Size | Purpose |
|---|---:|---:|---|
| `nvs` | `0x9000` | 24 KiB | Wi-Fi, pairing, settings, counters |
| `otadata` | `0xF000` | 8 KiB | OTA selection and rollback state |
| `phy_init` | `0x11000` | 4 KiB | PHY data |
| `nvs_keys` | `0x12000` | 4 KiB | Reserved for NVS encryption keys; encryption is disabled in development |
| `ota_0` | `0x20000` | 5 MiB | Application slot A |
| `ota_1` | `0x520000` | 5 MiB | Application slot B |
| `model` | `0xA20000` | 2 MiB | ESP-SR model pack |
| `assets` | `0xC20000` | 3.75 MiB | Retained factory/future UI assets; not mounted by the procedural UI |
| `coredump` | `0xFE0000` | 128 KiB | Bounded crash dump |

NVS namespaces:

```text
board          optional retained factory UUID
bobe_identity  generated stable device UUID
bobe_net       endpoint, trust-key ID, controller epoch, active-slot marker
bobe_tls       development CA, client certificate, and private key
bobe_clock     last authenticated wall-clock floor
```

Raw ambient audio is never written to flash.

SNTP updates the live clock but is not trusted for persistence. The firmware
writes a bounded clock floor only after a successful authenticated BodyLink
session and ignores floors outside the supported absolute time window.

Because flash encryption is disabled on this development unit, the client
private key stored in `bobe_tls` is recoverable by physical flash access. That
storage model is not suitable for production.

## 13. OTA and rollback

The partition table and firmware implement signed A/B OTA. An authenticated
gateway sends a canonical P-256 signed `update.offer`; firmware validates the
hardware model, monotonic build number, trusted-time window, HTTPS URL, image
size, and SHA-256 before streaming to the inactive slot. It selects the slot
only after ESP image validation. A new image must reach an authenticated
BodyLink session within ten minutes of boot. Session readiness confirms the
image; expiry marks it invalid and immediately reboots into the previous slot.

Power and lifecycle gates reject updates while busy, below 50% battery when not
charging, or before trusted time is established. `update.state` reports
acceptance, failure, progress state, and reboot. The gateway-side offer producer
loads only a pre-signed envelope; private signing keys remain offline.

Production requires signed images, Secure Boot v2, flash encryption, and
anti-rollback. Those irreversible controls are not burned during development;
[release security](release-security.md) describes their enrollment.

Factory full-flash restoration is separate from application rollback.

## 14. Security model

The current physical unit is development hardware:

- Secure Boot is disabled.
- Flash encryption is disabled.
- its factory HMAC key is read-protected and must not be repurposed casually.

Development requirements:

- validate the WSS server certificate;
- reject plaintext non-loopback endpoint URIs unless an explicit development
  configuration enables them;
- never log Wi-Fi passwords, keys, certificates, tokens, or audio payloads;
- keep provider, Copilot, MCP, and broad runtime credentials off-device;
- keep the current automatic recovery provisioning window time-bounded;
- require physical presence before provisioning in a production policy;
- bound every remote payload before allocation;
- expose an unmistakable microphone and connectivity state.

Production requirements:

- per-device certificate and revocation;
- mutual TLS;
- encrypted NVS;
- Secure Boot v2 and flash encryption in release mode;
- signed A/B OTA and anti-rollback;
- credential rotation and ownership transfer;
- a real hardware microphone power cutoff on future product hardware.

A lone long press of the volume-down button toggles a local microphone
mute. While muted, policy stops active capture and wake monitoring, rejects new
button and wake-word capture requests, ignores remote face overrides, and shows
the built-in muted expression alongside a queued hint naming the control that
undoes it. The mute is persisted under `mic_muted` and restored on the next
boot. This is a software privacy control, not a hardware microphone power
cutoff.

Credential rotation uses dual claim/TLS slots. A strictly newer controller epoch
is written to the inactive slot, atomically activated, and followed by retirement
of the previous slot. Interrupted retirement is
resumed idempotently when provisioning next starts. An authenticated
`claim.revoke` invalidates claim metadata, removes both credential slots, stops
BodyLink, and returns the body to bounded Security 2 provisioning. A ready claim
without a valid active-slot marker fails closed.

## 15. Offline behavior

When BoBe or Wi-Fi is unavailable, the body:

- stops remote capture and playback;
- shows an offline or sleeping-hub expression;
- retains persisted volume, selected face theme, microphone mute, battery safety,
  shutdown, and deterministic local animation;
- reconnects with bounded backoff;
- does not buffer ambient audio for later upload;
- does not fall back to an unrelated cloud assistant.

## 16. Health and observability

`health.report` carries bounded numeric metadata;
[BodyLink V1](bodylink-v1.md#controls-and-telemetry) shows its fields. It does
not include:

- raw audio;
- transcripts by default;
- Wi-Fi credentials;
- certificates or private keys;
- provider credentials;
- long-term personal memory.

Battery and charge state are sent separately through `battery.state`.

## 17. BoBe-side integration

BoBe's [`docs/architecture.md`](../../../../docs/architecture.md) describes the
runtime side of BodyLink. The wire contract's
[speech placement](bodylink-v1.md#speech-placement) shows how body audio
reaches speech recognition and how replies return as speaker PCM.

## 18. Autonomous validation architecture

Production lifecycle decisions pass through a pure `bobe_policy` reducer:
typed ingress plus explicit time/random inputs produce a bounded effect batch.
The ESP/FreeRTOS adapter executes effects and returns typed results. Host tests
therefore exercise the same policy used by physical callbacks rather than a
parallel mock state machine.

The display's widget renderer is likewise shared between the ESP LCD adapter
and an LVGL 9.5 host test. The host freezes phase/tick/scroll and compares
canonical 280x240 RGB565 hashes. A successful logical render does not prove the
write-only ST7789 panel.

The device only ever runs the normal product image, and validation observes it
from outside. The firmware therefore has to state what it does through ordinary
production logging. Every profile emits an identity line
(`device=... boot=... firmware=... project=... elf=...`), periodic health lines
and one physical button line per debounced edge
(`physical button event_id=... button=... action=... monotonic_us=...`). Each
compile-only profile gate asserts that the button evidence marker is present.

[`hil/README.md`](../hil/README.md) and [`hil-toolchain.md`](hil-toolchain.md)
describe the fixture topology, tool versions and safety boundary.

## 19. Verification rules

Build and test paths compile only; flashing is a separate, explicit command:

- `tools/idf.sh build` compiles only.
- Host tests compile protocol/state code with the Mac toolchain.
- No `tools/check.sh` gate flashes or actuates hardware.
- Flashing is `tools/bobe_hil.sh device flash` (or `tools/flash_app_verified.sh`
  for the fixed `ota_0` path); [`tools/README.md`](../tools/README.md)
  describes their checks.
- Keep owner-managed backups intact.

Build support does not qualify battery reporting, power and thermal behavior,
multi-day reliability, OTA recovery, acoustics, production enrollment or BLE
on a particular device. Use the [SDK validation guidance](../../../docs/validation.md)
before making hardware claims.

## 20. References

- WB-12 recovered hardware: [`README.md`](../README.md)
- BoBe physical design: BoBe's
  [`docs/physical-bobe.md`](../../../../docs/physical-bobe.md)
- BoBe platform authority and current implementation: BoBe's
  [`docs/architecture.md`](../../../../docs/architecture.md)
- ESP-IDF releases: <https://github.com/espressif/esp-idf/releases>
- ESP-IDF programming guide:
  <https://docs.espressif.com/projects/esp-idf/en/v6.0.2/esp32s3/>
- ESP Component Registry: <https://components.espressif.com/>
- ESP-SR: <https://github.com/espressif/esp-sr>
- LVGL 9.5: <https://github.com/lvgl/lvgl/tree/v9.5.0>
- RFC 6455 WebSocket: <https://www.rfc-editor.org/rfc/rfc6455>
- RFC 8446 TLS 1.3: <https://www.rfc-editor.org/rfc/rfc8446>
