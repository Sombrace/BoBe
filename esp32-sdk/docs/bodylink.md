# BodyLink

BodyLink connects a device to the BoBe daemon. The device side is
[`bobe_protocol`](../components/bobe_protocol) and
[`bobe_transport`](../components/bobe_transport), the daemon side
`BoBeService/src/body/`; unlisted limits are in those sources, and
`scripts/check-bodylink-contract.sh` checks their shared constants against the
Mac app and the WB-12 test server.

## Connection

| Item | Value |
|---|---|
| URL | `wss://HOST:PORT/body/v1`; the daemon's default port is 8767 |
| Subprotocol | `bobe.body.v1` |
| TLS | Mutual. The device verifies the server against the CA in its claim; the daemon accepts a client certificate only if its SHA-256 matches an enrolled endpoint |
| Frames | Text frames carry JSON control messages; binary frames carry media |
| Heartbeat | WebSocket ping and pong. The daemon closes a connection silent for 35 s; the SDK pings every 10 s |

A new connection from a device replaces its old one and starts with a new
`hello`. Open audio streams and unsent audio are discarded, never replayed. The
daemon serves only TLS.

mDNS service `_bobe-body-hub._tcp` carries TXT `pv=1`, `path=/body/v1`, `tls=1`,
`auth=mtls-pinned` and `kid` (the claim's trust key ID). Discovery only finds a
host; TLS authenticates it.

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

The daemon rejects unknown capability fields and accepts a minor only within the
enrolled `--protocol-min` and `--protocol-max`. `welcome` carries `protocol_minor`,
`session_id`, `connection_generation`, `heartbeat_interval_ms` (10000; the SDK
accepts 1000–60000) and `controller_epoch`. The device disconnects unless the
epoch equals its claim's and the minor is at least its own.

Grants are the JSON passed to `bobe endpoint enroll --capabilities`. A minor-1
endpoint enrolled with `{}` may use voice; otherwise every requested capability
among `microphone`, `speaker`, `text_input`, `text_output`, `actions` and
`sensors` must be granted `true`, or the connection is refused. Changing or
revoking an enrollment disconnects the device.

## Control messages

Every message is UTF-8 JSON of at most 4096 bytes with `type` and
`protocol_major: 1`. Malformed messages and route mismatches close the
connection, except late capture messages for a lease the daemon ended; the SDK
also rejects duplicate keys. The daemon closes the
connection on unknown types; the SDK ignores them. "Route" is the `lease_id`,
`turn_id` and `stream_id` of the active voice lease and stream.

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
| `health.report` | Status fields | The daemon reads `build_number`, `lifecycle` and `turn`, to decide on update offers |
| `battery.state` | `percent`, `charging` | Battery level and charging, used to retry an update offer |
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

`expression` is `booting`, `unprovisioned`, `connecting`, `idle`, `attentive`,
`listening`, `thinking`, `speaking`, `happy`, `concerned`, `muted`, `offline`,
`updating`, `error` or `sleeping`. A face with `lease_id` and `turn_id` applies
only to that turn, a newer `state_version` replaces an older one, and a non-zero
`ttl_ms` returns to the local face. The SDK ignores faces while muted.

`theme_id` is `warm-aperture`, `warm-lintel`, `warm-ledge`, `kettle-tall-bright`,
`kettle-soft-bowl`, `solar-wide-awake` or `solar-crown`, set with
`bobe config set body.theme`. Its `state_version` restarts every session.

## Voice turns

One device at a time holds the voice lease.

1. Device: `capture.request`, then no audio until `capture.granted`. Releasing
   the button before the grant sends `capture.cancel`.
2. Device: `audio.capture.open`, microphone frames, `audio.capture.close`.
3. Daemon: runs the turn and sends `turn.state` and `face.set`.
4. Daemon: `audio.playback.open`; after `audio.playback.ready`, speaker frames
   within the reported credit; then `audio.playback.close`.
5. Device: `audio.playback.drained` or `audio.playback.failed`. The daemon ends
   the turn with `face.set` `idle` and `turn.state` `idle`.

`lease_ttl_ms` starts a watchdog that each phase's progress extends, up to 10
minutes per turn. Among simultaneous `wake_word` requests the daemon picks one by
the optional `wake` scores (`confidence`, `snr_db`, `energy_dbfs`); an
`action_button` request wins, and a granted `wake_word` lease that has not
opened capture ends with `turn.state` `idle`.

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

Payloads are 20 ms of mono little-endian PCM16: 320 microphone samples at 16 kHz
(640 bytes) or 480 speaker samples at 24 kHz (960 bytes). Sequences count per
stream without gaps unless a frame is flagged as a discontinuity.
`played_samples` counts whole speaker frames.

## Text and actions

Each human request gets a new UUID `request_id`. `text` is 1–1024 bytes;
`action` is 1–64 letters, digits, `_`, `-` or `.`; `label` is up to 128 bytes.
Each device has its own conversation channel and runs one request at a time.

| `status` | Meaning |
|---|---|
| `accepted` | Started. Answering `input.cancel`, `code` is `cancellation_requested` |
| `running` | Already running under this ID |
| `completed`, `failed`, `cancelled` | Finished |
| `rejected` | Not run; `code` explains, for example `not_supported`, `invalid_text`, `invalid_action`, `busy`, `busy_or_conflict`, `unsafe_retry` or `not_active` |

`replayed: true` means the daemon has seen the ID. Resending a completed request
returns its stored answer and reusing an ID for different content is rejected.
Resending a failed or interrupted request returns `unsafe_retry`; ask the person
before retrying with a new ID.

With `text_output`, the answer arrives as `display.text` chunks of at most 512
bytes. `sequence` starts at 0 with `clear: true`; a last empty chunk has
`final: true`. NUL characters are replaced. Show text as data; never execute it.
`bobe endpoint display DEVICE_ID --text "..."` (`POST /endpoints/{id}/display`)
sends owner text; `queued` means sent, not displayed.

## Sensors

`name` is 1–32 letters, digits, `_`, `-` or `.`; `value` is a finite number or a
boolean; `unit` is up to 16 bytes without control characters. Samples never
start a turn. Sequences increase per sensor within a connection; an identical
repeat is acknowledged again and an older one is refused. The daemon answers
`sensor.ack`, or `error` with `stale_sensor_sequence`, `sensor_rate_limited`
(about ten samples a second) or `sensor_limit` (16 names per device).

`bobe endpoint sensors DEVICE_ID --json` (`GET /endpoints/{id}/sensors`) reads
the last 5 minutes of readings, which the daemon keeps only in memory;
`observed_at` is the receive time.

## Signed updates

`update.offer` has exactly `type`, `protocol_major`, `manifest` and `signature`.
`manifest` is a canonical JSON string with exactly `schema` (1), `device_model`,
`version`, `build_number`, `image_size`, `image_sha256`, `url` (HTTPS), `key_id`,
`not_before` and `expires_at`; `signature` is the base64 DER ECDSA
P-256/SHA-256 signature of those bytes. The device verifies the signature before
reading any field, then requires its own model, a higher `build_number` than
its own, an image that fits its update slot and a trusted clock inside
`[not_before, expires_at)`. A failed offer closes the connection.

The daemon sends its configured offer when `health.report` shows an older
`build_number`. The SDK installs only while online and idle with
the battery at 50 % or charging, and reports `update.state` `accepted` then
`rebooting`, or `failed` with `reason` `busy`, `battery_unknown`, `low_battery`
or `install_failed`. After `busy`, `battery_unknown` or `low_battery` the
daemon offers again with backoff (30 s doubling to 15 minutes), sooner once
the device reports idle or charging. After `accepted`, `install_failed` or a
reconnect without an answer, it waits for a daemon restart or a new
enrollment. A new image that does not reach an authenticated session
within 10 minutes rolls back. The
[WB-12 guide](../examples/wb12/README.md#signed-updates) shows how to create an
offer.
