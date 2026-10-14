# Independent physical audio observer

The observer is a planned Raspberry Pi appliance that will record acoustic
evidence outside the untrusted Mac session, as the authority for one claim:
`physical-bodylink-audio-loop`. That claim covers the WB-12 microphone, BodyLink
media, playback path, speaker and a bounded fixture action, not speech
recognition, model routing, replies or synthesis. Until it is deployed, no live
scenario can claim that audio loop.

```bash
tools/bobe_hil.sh observer status
tools/bobe_hil.sh observer probe
```

`status` validates the local configuration without connecting: the Ed25519 mTLS
CA, server and client chain, the exact server IP SAN, pinned server and client
SPKIs, the client key match and the persistent counter. `probe` adds a TLS 1.3
status exchange with hostname, CA and leaf-SPKI verification under a 12-second
deadline.

## Trust model

The Mac user session and any agent in it can read, replace or fabricate media, so
macOS capture never authorizes a physical pass. The observer trusts the
physically controlled Pi and its root administrator, the reviewed deployment and
its root-only evidence key, an operator-signed unexpired qualification record,
and the audio interface, wiring and geometry that record names. It does not
resist a compromised Pi root.

An offline trust root signs the observer's evidence-key certificate (purpose
`bobe-independent-observer-evidence`) and the qualification record. Each run
combines a host nonce and request ID with an observer nonce, boot ID and strictly
increasing counter; the observer signs the scenario contract, identities,
transcripts, media manifest, fixed-point metrics and cleanup result, and the host
verifies signature, freshness, qualification and artifact bytes before keeping
them. Mock observers use a separate trust root and certificate purpose that
production verification rejects.

## Evidence contract

`bobe_hil/observer_evidence.py` verifies a pass only with exact observer,
deployment, qualification, device, boot, ELF, client-key, USB and Pico bindings,
both nonces, a new counter, complete artifacts, met thresholds and verified
cleanup. Signed failures stay verifiable but never pass.

| Artifact | Contents |
|---|---|
| `uplink-stimulus.pcm` | Pi-to-WB-12 microphone stimulus, 48,000 samples at 48 kHz |
| `observer-inputs.pcm` | Synchronized witness and electrical-tap capture, 384,000 frames of two-channel 48 kHz |
| `bodylink-microphone.pcm` | WB-12 microphone PCM received over BodyLink, 50–500 frames of 20 ms at 16 kHz |
| `bodylink-speaker.pcm` | PCM sent to WB-12 playback, 150 frames of 20 ms at 24 kHz |
| `bodylink-transcript.jsonl` | Ordered control and media metadata |
| `fixture-transcript.jsonl` | Ordered Pico identity, action and release |

Artifacts are content-addressed and bounded, and hashes without bytes are
insufficient. PCM is little-endian `application/vnd.bobe.pcm-s16le` whose size
matches its channel and sample counts; big-endian `audio/L16` is forbidden.
Signed metrics and thresholds are integers in fixed units.

## Service scaffold

`bobe_observer` is the Pi-side protocol and CLI:

```bash
/opt/bobe-observer/venv/bin/python -I -m bobe_observer check-config
/opt/bobe-observer/venv/bin/python -I -m bobe_observer serve
```

It reports `ready: false`, advertises only `status`, returns `NOT_IMPLEMENTED`
for the audio loop, has no mock mode and never advances the counter. It accepts
only TLS 1.3 with a CA-valid client certificate whose SPKI matches the
configuration. systemd supplies TLS material as credentials and runs it as
`bobe-observer` with no capabilities, a private device namespace,
`DevicePolicy=closed`, no `DeviceAllow` entries and a mode-0700 state directory,
so it cannot reach ALSA or the Pico until reviewed device policy and the native
runner exist. `artifact-read` serves only the six artifact names for a completed
current-counter run, with a signed transfer nonce, per-name size limits and at
most 4,096 bytes per chunk. The Mac-side artifact retention transaction is not
implemented.

The network API will accept only `status` and `physical-bodylink-audio-loop` over
pinned mTLS; it never exposes SSH, a shell, raw ALSA, camera options, GPIO, paths
or thresholds.

## Host configuration

Under the ignored `private/hil/observer/`: `observer.json`, `observer-ca.pem`,
`observer-server-cert.pem`, `mac-client-cert.pem`, mode-private
`mac-client-key.pem`, `evidence-public.pem`, `evidence-certificate.json`,
`qualification.json` and, after the first accepted run, `counter.json`. The
`*.example.json` files here are unsigned placeholders.
