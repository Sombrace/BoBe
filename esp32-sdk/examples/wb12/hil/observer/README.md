# Independent physical audio observer

The planned observer is a physically trusted Raspberry Pi appliance that
separates acoustic evidence from the untrusted macOS agent session. It is the
future authority for the narrow claim:

`physical-bodylink-audio-loop`

That claim covers the WB-12 microphone, BodyLink media transport, playback
path, physical speaker and bounded fixture action. It does not prove production
STT, model routing, response generation, TTS quality or the separate `BoBe`
runtime.

The host tool reports local configuration state and can probe the appliance:

```bash
tools/bobe_hil.sh observer status
tools/bobe_hil.sh observer probe
```

`status` validates the local configuration, the Ed25519 mTLS CA/server/client
chain, the exact server IP SAN, the pinned server/client SPKIs, the client
private-key match and the persistent counter, without connecting. `probe`
additionally performs a TLS 1.3 status exchange with hostname, CA and
leaf-SPKI verification under one 12-second monotonic deadline.

## Threat model

The Mac user and coding agent are untrusted evidence transports. They may read,
replace or fabricate user-owned media, so macOS CoreAudio/AVFoundation output
cannot authorize a physical pass. Root does not bypass microphone/camera TCC,
and a user-session helper is not an independent observer.

The first observer version trusts:

- the physically controlled Raspberry Pi and its root administrator;
- the reviewed observer deployment and root-only evidence key;
- an operator-signed, unexpired qualification record;
- the exact audio interface, wiring, fixture and geometry in that record.

It does not claim resistance to compromised Pi root. Secure boot, immutable
root filesystems and protected key storage can reduce that risk, but a TPM or
secure element alone cannot prevent compromised software from signing false
measurements. Any future stronger claim must identify and verify the booted
deployment, not merely certify a public key.

## Fixed topology

Use one class-compliant 2-input/2-output USB audio interface:

1. Pi output 1 drives the calibrated stimulus speaker aimed at the WB-12
   microphone.
2. Pi input 1 records the calibrated witness microphone.
3. Pi input 2 records a protected electrical tap from the WB-12 speaker path.
4. The observer receives the WB-12 microphone stream through BodyLink.
5. The Pico 2 fixture performs the bounded action press and reports release.

Both input channels must be opened as one interleaved ALSA `hw:` stream before
actuation. The authoritative service must use exact USB identity, channel map,
`S16_LE` sample format and gain; it must reject PipeWire, `dmix`, resampling, AGC,
caller-selected devices and caller-selected paths.

The downlink electrical tap distinguishes WB-12 speaker output from leakage
from the fixture stimulus speaker. Silence/null and crosstalk tests are part of
qualification. Optional CSI camera evidence is a separate visual subclaim and
must not silently weaken or strengthen an audio result.

## Trust and freshness chain

1. The offline manifest trust root signs an observer evidence-key certificate
   with purpose `bobe-independent-observer-evidence`.
2. The same offline authority signs a qualification record binding observer,
   deployment, audio USB identity, exact wiring, calibration, gains, thresholds
   and expiry.
3. The macOS host creates a fresh 256-bit nonce and request ID.
4. The observer contributes a fresh 256-bit nonce, boot ID and strictly
   increasing persistent run counter.
5. The observer signs the exact scenario contract, identities, transcripts,
   media manifest, fixed-point metrics and cleanup result.
6. The host verifies that signature, its freshness, the qualification record
   and the downloaded artifact bytes before retaining them as run evidence.
7. Offline verification rechecks the observer signature. Only the separate
   machine's signature says what it physically measured; retaining the bytes
   locally does not upgrade stale or invalid observer evidence.

Mock observers use a separate test trust root and a different certificate
purpose. Production verification rejects mock certificates before examining
their claimed status.

## Signed artifact set

The production pass contract requires exactly:

| Artifact | Meaning |
|---|---|
| `uplink-stimulus.pcm` | Fresh Pi-to-WB microphone stimulus |
| `observer-inputs.pcm` | Synchronized witness/electrical two-channel capture |
| `bodylink-microphone.pcm` | Exact WB microphone PCM received over BodyLink |
| `bodylink-speaker.pcm` | Exact PCM sent to WB playback |
| `bodylink-transcript.jsonl` | Ordered control/media metadata transcript |
| `fixture-transcript.jsonl` | Ordered Pico identity/action/release transcript |

Artifacts are content-addressed, bounded and retrievable. Hashes without the
matching bytes are insufficient. PCM sizes must match channel and sample
counts. Every PCM media type is explicitly little-endian
`application/vnd.bobe.pcm-s16le`, matching BodyLink and ALSA `S16_LE`; generic
big-endian `audio/L16` is forbidden. Metrics and thresholds are integers in
fixed units; floating-point values and non-finite numbers are forbidden in the
signed envelope.

A pass requires these fixed media windows:

- exactly 48,000 samples (1 second) of 48 kHz uplink stimulus;
- 50–500 complete 20 ms WB microphone frames (1–10 seconds at 16 kHz);
- exactly 150 complete 20 ms downlink frames (3 seconds at 24 kHz);
- exactly 384,000 frames (8 seconds) of synchronized 48 kHz two-channel
  observer input.

`bobe_hil/observer_evidence.py` implements the verifier-first contract. It
requires exact observer/deployment/qualification/device/boot/ELF/client-key/
USB/Pico bindings, dual nonces, a new run counter, complete artifacts, threshold
success and verified cleanup before returning a physical pass. Signed failures
remain verifiable but never authorize a pass.

## Selected platform tools

- Raspberry Pi OS Lite arm64 from an exact dated image and published SHA-256;
- APT repository signature verification with no unsigned repositories;
- ALSA `libasound` for the authoritative synchronized stream;
- `alsa-utils` only for setup/qualification diagnostics;
- official `rpicam-apps` for an optional CSI camera;
- systemd for a dedicated service user, device ACLs and service sandboxing.

Do not expose SSH, shell, raw ALSA, arbitrary camera options, GPIO, file paths or
thresholds to the Mac agent. The future network API accepts only fixed
`status` and `physical-bodylink-audio-loop` operations over pinned mTLS.

## Service scaffold

`bobe_observer` defines the Pi-side service protocol and CLI:

```bash
/opt/bobe-observer/venv/bin/python -I -m bobe_observer check-config
/opt/bobe-observer/venv/bin/python -I -m bobe_observer serve
```

The backend always reports `ready: false`, advertises only `status` and
returns `NOT_IMPLEMENTED` for `physical-bodylink-audio-loop`. It has no mock
mode and never advances the run counter.

The service accepts TLS 1.3 only, requires a CA-valid client certificate and
then compares its SPKI to the fixed root configuration. systemd provides TLS
and future evidence keys as service credentials. The service runs as
`bobe-observer` with no capabilities, a private device namespace,
`DevicePolicy=closed`, no `DeviceAllow` entries and an owned mode-0700 state
directory. It cannot access ALSA or Pico hardware until reviewed device
identity/udev policy and the native runner are added.

`service-config.example.json` is a placeholder.

The shared protocol implements content-addressed artifact transfer for an
already completed current-counter run. `artifact-read` accepts only the six
fixed artifact names, a signed transfer nonce/run counter, a name-specific
maximum, and at most 4,096 bytes per chunk. The Pi reads service-owned regular
files through stable descriptors and returns chunk/full hashes.

The Mac-side artifact transfer and retention transaction is not implemented.

Host configuration lives under the gitignored repository-local
`private/hil/observer/`:

- `observer.json`;
- `observer-ca.pem`;
- `observer-server-cert.pem`;
- `mac-client-cert.pem` and mode-private `mac-client-key.pem`;
- `evidence-public.pem` and `evidence-certificate.json`;
- `qualification.json`;
- `counter.json` after the first accepted run.

`observer-config.example.json` and `qualification.example.json` are unsigned
placeholders.

Primary references:

- <https://www.raspberrypi.com/software/operating-systems/>
- <https://www.raspberrypi.com/documentation/computers/camera_software.html>
- <https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#secure-boot>
- <https://github.com/alsa-project/alsa-utils>
- <https://manpages.debian.org/stable/apt/apt-secure.8.en.html>
- <https://www.rfc-editor.org/rfc/rfc9334.html>

## Bring-up

- select and characterize the USB audio interface, microphone, source speaker
  and protected electrical tap;
- measure crosstalk, SPL/SNR, latency, polarity, clipping, dropout and null
  behavior;
- qualify the Pico contacts and exact fixture timing;
- provision and measure the observer deployment;
- sign the real qualification record;
- verify same-device BodyLink certificate SPKI, WB boot ID and reviewed ELF.

Record the measured values in the qualification document when the appliance is
built.
