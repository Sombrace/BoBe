# BoBe on iPhone

The source tree includes a native iOS 26 app. It connects to BoBe on a Mac or
Linux computer. The phone does not run a second agent or hold a separate memory.
Your BoBe must be awake and reachable.

## What the app does

- Shows streamed conversation and questions from BoBe.
- Keeps the phone's own conversation thread, with the same shared memory and goals.
- Shows goals, schedules and recent work; lets you add, pause or complete goals.
- Shows searchable memory and lets you ask BoBe to forget something.
- Converts hold-to-talk speech to text on the phone.

The interface supports English, Greek, Chinese, German, Spanish, Portuguese,
Korean, Japanese and French.

## Pair the phone

On a Mac with a managed-local BoBe, open **Settings > Devices > Pair iPhone**.
If needed, allow iPhone access so the runtime can listen beyond the Mac itself.
An attached or remote runtime must be made reachable by its owner.

For a standalone runtime, use:

```sh
bobe device pair --kind phone
```

Scan the QR code with the app, open its `bobe://pair` link, or enter the runtime
address and the six-digit code. The QR/link carries the runtime's key fingerprint
as well as its addresses. Manual entry trusts the key presented on the first
pairing. Subsequent HTTPS requests and the event stream must match the paired key.

Each phone gets a credential with phone access, not provider or runtime
administration permissions. The credential stays in the device Keychain.
Remove the pairing in the phone's Settings or revoke it from the owning runtime.

## Reconnection and speech

The app tries the paired runtime's known addresses and resumes its conversation
after reconnecting. It disconnects its live stream in the background and connects
again when opened. A runtime that is stopped or unreachable cannot answer.

Hold-to-talk requires Microphone and Speech Recognition permissions. Once voice
is allowed, the app prepares Apple's language assets. It recognizes speech with
Apple's `SpeechAnalyzer`, using `SpeechTranscriber` where available and
`DictationTranscriber` otherwise. Recognition uses the phone's language where
supported, otherwise English. Only the resulting words are sent, as a chat
message.

## Current limitations

Hold-to-talk is speech input, not complete two-way phone voice. The app
does not provide brain-synthesized reply playback or push notification delivery.

Camera scanning, microphone capture and language-asset preparation require
qualification on a physical iPhone. The simulator does not supply the speech
engine used by this app.

The app is defined by `BoBeiOS/project.yml`. Building it requires Xcode and the
iOS pairing library from `scripts/tasks/build-pairing-ios.sh`; that script builds
both the device and Apple Silicon simulator variants. Installing on a physical
phone also requires the appropriate signing setup.
