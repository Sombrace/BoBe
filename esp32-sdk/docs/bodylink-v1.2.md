# BodyLink minor 2: general device interactions

Transport is `bobe.body.v1` over mutually authenticated WSS. Major version
1 and binary audio framing are unchanged. Minor 2 adds non-audio devices and
explicitly authorized interactions. The voice profile uses minor 1.

## Hello and grants

Capabilities add `text_input`, `text_output`, `actions`, and `sensors` booleans.
Optional `text_display` gives `columns` and `rows` (1–240). `display`
describes a face renderer in pixels. An absent display is valid. Non-audio
devices advertise zero playback credit.

Owner enrollment must allow minor 2 and grant the requested capabilities.
Approved `capabilities` are distinct from `observed_capabilities` in the endpoint
API. Enrollment changes disconnect the old session so permissions take effect.

All messages contain `protocol_major: 1`. Control frames are at most 4096 UTF-8
bytes; firmware rejects excessive nesting before recursive JSON parsing.

## Human input

| Type | Fields | Meaning |
|---|---|---|
| `input.text` | `request_id`, `text` | Committed text entry, at most 1024 UTF-8 bytes |
| `input.action` | `request_id`, `action`, `label` | Action ID (1–64 identifier bytes) and label (up to 128 bytes) |
| `input.cancel` | `request_id` | Cancel this connection's active request |
| `input.result` | `request_id`, `status`, `code`, `replayed` | Acceptance or terminal result |

Request IDs use the 36-character UUID representation. Status values are
`accepted`, `running`, `completed`, `cancelled`, `failed`, and `rejected`.
Rejection codes include `not_supported`, `invalid_text`, `invalid_action`,
`busy`, `busy_or_conflict`, and `unsafe_retry`.

Local send success is not an acceptance receipt. Preserve the request ID until
resolved. Reusing it for different content is rejected. Completed replay returns
the recorded answer without running the agent again. Do not silently assign a
new ID to an interrupted request and repeat possible side effects.

Keyboard input and actions share the daemon's normal conversation, tool policy,
and single-turn admission. Sensor reports do not enter that queue.

## Text output

`display.text` contains `request_id`, `sequence`, `clear`, `final`, and `text`.
Sequence starts at zero; the first frame clears the prior view. Chunks contain
at most 512 UTF-8 bytes and may be smaller to fit the advertised frame limit.
A final empty frame marks completion. NUL is replaced before device delivery.

`bobe_text_view` detects gaps and duplicates and maintains bounded UTF-8 storage.
A new request's clear frame replaces the view; late chunks from other requests
are ignored. Render from the application's UI task.

Owners can send text directly with `bobe endpoint display DEVICE_ID --text "..."`.
The API is `POST /endpoints/{id}/display` with `{"text":"..."}`. Its `queued`
result describes transport queuing, not proof of physical rendering.

## Sensors

`sensor.sample` contains `name`, `sequence`, `value`, and optional `unit`. Names
contain 1–32 ASCII letters, digits, underscores, dots, or hyphens. Values are
finite numbers or booleans. Units contain at most 16 UTF-8 bytes without ASCII
control characters. `sensor.ack` returns the accepted name and sequence.

Sequences increase per sensor within a connection. Exact duplicates are
idempotent; stale sequences are rejected. Reconnection permits rebooted devices
to restart their sequence. The daemon allows bursts of ten samples, refills at
ten/second, and retains at most sixteen names per endpoint.

Read observations through `bobe endpoint sensors DEVICE_ID --json` or
`GET /endpoints/{id}/sensors`. `observed_at` is receipt time, not a claim about
physical measurement time. Readings remain available during device sleep for
five minutes. They are ephemeral across daemon restarts.
