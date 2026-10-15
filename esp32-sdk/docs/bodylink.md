# BodyLink

BodyLink connects a device to the BoBe daemon. The device side is
[`components/bobe_protocol`](../components/bobe_protocol) and
[`components/bobe_transport`](../components/bobe_transport); the daemon side is
`BoBeService/src/body/`. `scripts/check-bodylink-contract.sh` in the repository
root checks that the shared constants match. Limits not listed here are in those
sources.

## Connection

| Item | Value |
|---|---|
| URL | `wss://HOST:PORT/body/v1`; the daemon's default port is 8767 |
| Subprotocol | `bobe.body.v1` |
| TLS | Mutual. The device verifies the server against the CA in its claim; the daemon accepts a client certificate only if its SHA-256 matches an enrolled endpoint |
| Frames | Text frames carry JSON control messages; binary frames carry media |
| Heartbeat | WebSocket ping and pong. The daemon closes a connection that is silent for 35 s; the SDK pings every 10 s |

A device keeps one connection. A new connection from the same device replaces
the old one. After a reconnect the device sends a new `hello`; open audio streams
and unsent audio are discarded, never replayed.

The daemon advertises `_bobe-body-hub._tcp` over mDNS with TXT `pv=1`,
`path=/body/v1`, `tls=1`, `auth=mtls-pinned` and `kid` (the claim's trust key
ID). Discovery only finds a host; TLS still authenticates it. The firmware
accepts `ws://` only when built with `CONFIG_BOBE_ALLOW_INSECURE_WS`, and the
daemon serves only TLS.

## Handshake and grants

The device sends `hello` within 5 s of connecting. The daemon answers `welcome`,
then `theme.set` if the device has a face display.

| `hello` field | Meaning |
|---|---|
| `device_id` | Enrolled endpoint ID; must belong to the client certificate |
| `boot_id` | 32 hex characters, new on every boot |
| `firmware`, `hardware` | Version and model strings, up to 64 bytes |
| `protocol_minor` | 1 for a voice-only body (microphone and speaker, no minor-2 capability), otherwise 2 |
| `capabilities` | `microphone`, `speaker`, `battery`, `aec`, `wake_word`; minor 2 adds `text_input`, `text_output`, `actions`, `sensors`; optional `display` `{width, height, face_schema: 1}` and `text_display` `{columns, rows}` (1–240, needs `text_output`) |
| `limits` | `max_control_bytes` (1024–4096) and `max_playback_credit_ms` (at most 300; at least 20 with a speaker) |

The daemon rejects unknown capability fields. It accepts the minor when it lies
within the endpoint's enrolled `--protocol-min` and `--protocol-max`. `welcome`
carries `protocol_minor`, `session_id`, `connection_generation`,
`heartbeat_interval_ms` (10000; the SDK requires 1000–60000) and
`controller_epoch`. The device closes the connection unless the epoch equals the
one in its claim and the minor is at least its own.

Owner grants are the JSON passed to `bobe endpoint enroll --capabilities`. A
minor-1 endpoint enrolled with `{}` may use voice. Otherwise every requested
`microphone`, `speaker`, `text_input`, `text_output`, `actions` and `sensors`
capability must be granted `true`, or the connection is refused. What the
device reports is stored separately as `observed_capabilities` and never
expands a grant. Changing or revoking an enrollment disconnects the device.

## Control messages

Every JSON message has `type` and `protocol_major: 1` and is at most 4096 bytes
of UTF-8. Malformed messages and route mismatches close the connection, and the
SDK also rejects duplicate keys. The SDK ignores message types it does not know;
the daemon closes the connection on them. "Route" below means `lease_id`,
`turn_id` and `stream_id`, which must match the active voice lease and stream.

Device to daemon:

| Type | Fields | Meaning |
|---|---|---|
| `hello` | See above | Opens the session |
| `capture.request` | `request_id`, `stream_id`, `trigger`, `codec`, `sample_rate_hz`, `channels`, `frame_duration_ms`, optional `wake` | Asks for the voice lease. `trigger` is `action_button` or `wake_word`; the format must be `pcm_s16le`, 16000, 1, 20 |
| `capture.cancel` | `request_id`, `stream_id` | Withdraws a request or releases its lease; repeating it is harmless |
| `audio.capture.open` | Route, `codec`, `sample_rate_hz`, `channels`, `frame_duration_ms` | Microphone frames follow |
| `audio.capture.close` | Route, optional `reason` | No reason, `button_released` or `vad_end` ends capture; any other reason abandons the turn |
| `audio.playback.ready` | Route, `credit_ms` | Ready for speaker frames, with free credit in milliseconds |
| `audio.playback.ack` | Route, `played_samples`, `credit_ms` | Samples that have physically played, and free credit |
| `audio.playback.drained` | Route | Every accepted frame has played |
| `audio.playback.failed` | Route, `reason` | Playback stopped before draining |
| `input.text` | `request_id`, `text` | A finished text entry |
| `input.action` | `request_id`, `action`, `label` | A person used a named control |
| `input.cancel` | `request_id` | Cancels this connection's running request |
| `sensor.sample` | `name`, `sequence`, `value`, optional `unit` | One observation |
| `health.report` | Status fields | The daemon reads only `build_number`, to decide on update offers |
| `battery.state` | `percent`, `charging` | Battery level; the daemon only checks that `percent` is 0–100 |
| `button.event` | `boot_id`, `event_id`, `button`, `action`, `monotonic_us` | A physical button edge; the daemon accepts and ignores it |
| `update.state` | `phase`, `progress_percent`, optional `reason` | Update progress |

Daemon to device:

| Type | Fields | Meaning |
|---|---|---|
| `welcome` | See above | Session accepted |
| `capture.granted` | `request_id`, `stream_id`, `lease_id`, `turn_id`, `lease_ttl_ms` | Voice lease granted |
| `capture.denied` | `request_id`, `stream_id`, `reason` | Not granted, for example `another_endpoint_owns_voice_lease` or `speech_engine_unavailable` |
| `turn.state` | `lease_id`, `turn_id`, `state` | `waiting_for_runtime`, `thinking` or `speaking`; `aborted` or `idle` ends the turn |
| `face.set` | `state_version`, `expression`, optional `lease_id` with `turn_id`, `caption`, `accent_rgb`, `ttl_ms` | Desired face |
| `theme.set` | `state_version`, `theme_id` | Face theme |
| `audio.playback.open` | Route, `codec`, `sample_rate_hz`, `channels`, `frame_duration_ms` | Speaker stream: `pcm_s16le`, 24000, 1, 20 |
| `audio.playback.close` | Route | The last frame has been sent |
| `input.result` | `request_id`, `status`, `code`, `replayed` | Request outcome |
| `display.text` | `request_id`, `sequence`, `clear`, `final`, `text` | A chunk of response text |
| `sensor.ack` | `name`, `sequence` | Sample accepted |
| `error` | `code`, `message`, optional `lease_id` and `turn_id` | A refused sample or a failed turn |
| `update.offer` | `manifest`, `signature` | Signed firmware offer |

`face.set` `expression` is one of `booting`, `unprovisioned`, `connecting`,
`idle`, `attentive`, `listening`, `thinking`, `speaking`, `happy`, `concerned`,
`muted`, `offline`, `updating`, `error` or `sleeping`. The SDK applies only a
newer `state_version`, ignores faces while muted, shows `listening` or
`speaking` only in that turn state, and returns to its local face after a
non-zero `ttl_ms`. A face with `lease_id` and `turn_id` applies only to that
turn.

`theme_id` is one of `warm-aperture`, `warm-lintel`, `warm-ledge`,
`kettle-tall-bright`, `kettle-soft-bowl`, `solar-wide-awake` or `solar-crown`
(the owner sets it with `bobe config set body.theme`). The SDK applies a higher
`state_version`, restarts the count on every session and keeps the last theme
for the next boot.

## Voice turns

One device at a time holds the voice lease.

1. The device sends `capture.request` and sends no audio until
   `capture.granted`. Releasing the button before the grant sends
   `capture.cancel`.
2. The device sends `audio.capture.open`, microphone frames and
   `audio.capture.close`.
3. The daemon recognizes the speech, with its own engine or through the Mac app's
   speech adapter, runs the turn and sends `turn.state` and `face.set`.
4. The daemon sends `audio.playback.open`, waits for `audio.playback.ready`, sends
   speaker frames without exceeding the reported credit, then sends
   `audio.playback.close`.
5. The device reports `audio.playback.drained` or `audio.playback.failed`. The
   daemon ends the turn with `face.set` `idle` and `turn.state` `idle`.

`lease_ttl_ms` starts a watchdog that progress in each phase extends, up to 10
minutes per turn; the SDK enforces its own 10-minute limit. When several devices
send `wake_word` requests at once, the daemon picks one using the optional `wake`
scores (`confidence`, `snr_db`, `energy_dbfs`); an `action_button` request takes
precedence.

## Media frames

Each binary message is one frame: a 20-byte big-endian header and its payload.

| Offset | Size | Field |
|---:|---:|---|
| 0 | 1 | `protocol_major`, 1 |
| 1 | 1 | Kind: `0x01` microphone, `0x02` speaker |
| 2 | 2 | Flags: bit 0 marks a discontinuity; other bits are 0 |
| 4 | 4 | `stream_id`, non-zero |
| 8 | 4 | `sequence` |
| 12 | 8 | Monotonic timestamp in microseconds, not wall-clock time |

Payloads are 20 ms of mono PCM16, little-endian: 320 microphone samples at
16 kHz (640 bytes) or 480 speaker samples at 24 kHz (960 bytes). Sequence numbers
count per stream without gaps, except on a frame flagged as a discontinuity.
`played_samples` counts whole speaker frames.

## Text and actions

Give each human request a new UUID `request_id` and keep it until `input.result`
settles it. `text` holds 1–1024 bytes. `action` is 1–64 letters, digits, `_`,
`-` or `.`, and `label` is up to 128 bytes. Each device has its own conversation
channel and runs one request at a time.

| `status` | Meaning |
|---|---|
| `accepted` | Started. Answering `input.cancel`, `code` is `cancellation_requested` |
| `running` | Already running under this ID |
| `completed`, `failed`, `cancelled` | Finished |
| `rejected` | Not run; `code` explains, for example `not_supported`, `invalid_text`, `invalid_action`, `busy`, `busy_or_conflict`, `unsafe_retry` or `not_active` |

`replayed: true` means the daemon has seen the ID before. Sending a completed
request again returns its stored answer without running it again. Reusing an ID
for different content is rejected. Sending a failed or interrupted request again
returns `unsafe_retry`; ask the person before sending it with a new ID.

With `text_output`, the answer arrives as `display.text` chunks of at most 512
bytes. `sequence` starts at 0 with `clear: true`, and a last empty chunk has
`final: true`. NUL characters are replaced before sending. Show text as data;
never execute it. The owner can also send text with
`bobe endpoint display DEVICE_ID --text "..."` (`POST /endpoints/{id}/display`);
its `queued` result means sent, not displayed.

## Sensors

`sensor.sample` has a `name` of 1–32 letters, digits, `_`, `-` or `.`, a
`sequence`, a finite number or boolean `value`, and an optional `unit` of up to
16 bytes without control characters. Samples never start a conversation turn.
Sequences increase per sensor within a connection; an identical repeat is
acknowledged again and an older sequence is refused. The daemon answers
`sensor.ack`, or `error` with `stale_sensor_sequence`, `sensor_rate_limited`
(about ten samples a second) or `sensor_limit` (16 names per device).

Readings stay in daemon memory for 5 minutes, also while the device is
disconnected. Revocation, a privacy purge and a daemon restart clear them. Read
them with `bobe endpoint sensors DEVICE_ID --json` (`GET /endpoints/{id}/sensors`);
`observed_at` is the time the daemon received the sample.

## Signed updates

`update.offer` has exactly `type`, `protocol_major`, `manifest` and `signature`.
`manifest` is a canonical JSON string with exactly `schema` (1), `device_model`,
`version`, `build_number`, `image_size`, `image_sha256`, `url` (HTTPS), `key_id`,
`not_before` and `expires_at`. `signature` is the base64 DER ECDSA P-256/SHA-256
signature of the manifest bytes. The device checks the signature before it reads
any field, then requires its own model, a higher `build_number` than it runs, an
image that fits its update slot and a trusted clock inside
`[not_before, expires_at)`. An offer that fails closes the connection.

The SDK voice runtime installs an offer only while online and idle, with the
battery at 50 % or charging. It reports `update.state` `accepted` and then
`rebooting`, or `failed` with `reason` `busy`, `battery_unknown`, `low_battery`
or `install_failed`. The new image must reach an authenticated session within
10 minutes, or the device rolls back to the previous one. The daemon sends its
configured offer once per connection, when `health.report` shows an older
`build_number`. [Updates and release security](../examples/wb12/README.md#updates-and-release-security)
covers creating and serving an offer.
