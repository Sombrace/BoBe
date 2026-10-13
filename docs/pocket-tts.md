# Pocket TTS migration

BoBe's daemon speech engine is Pocket TTS. This first pack supports English,
with Alba (default), Azelma, Eponine, Fantine, Javert and Marius. Existing Kokoro
voice selections resolve to Alba. The Mac app migrates its saved daemon engine
selection, and the daemon advertises `server_pocket` on the existing
`bobe.voice.v2` connection. Other Pocket language packs are not installed or
advertised yet. Unsupported language/engine combinations fail during negotiation.

## Runtime and model

- Native Rust inference: `ptts` 0.4.0-rc.1 (MIT or Apache-2.0), CPU Q8.
- Pack: `pocket-tts-en-b6369a24-q8`, approximately 169 MB including six presets.
- Fixed English weights from Kyutai's original b6369a24 generation, converted
  to GGUF in the runtime author's recommended `davencyw/pocket-tts-ptts` layout.
  This is a pinned third-party conversion, not Kyutai's newer multilingual v3
  checkpoint. The conversion revision and every file's SHA-256 are recorded in
  `BoBeService/src/voice/install_artifacts.rs`.
- No Python interpreter, model-server process, microphone upload or cloning
  pipeline is needed for synthesis. All inference happens in the daemon.
- 0.5–2× speed uses MIT-licensed Signalsmith Stretch with pitch preservation.
  Building its native wrapper requires a C++ compiler and libclang (provided by
  Xcode on macOS; typically `clang` and `libclang-dev` on Linux).
- Recognition remains sherpa-onnx 1.13.8, using its pinned `static-no-tts-lib`
  distribution. The local sys-crate adaptation excludes eSpeak NG and Piper.
  macOS arm64/x86_64 and Linux x86_64 archives are available; other target
  architectures fail the build explicitly.

The native Pocket runtime is a release candidate. This branch does not establish
quality parity with the newest Kyutai models or production readiness on every
supported machine. Adding languages requires a compatible, independently pinned
model/tokenizer/voice pack and an updated capability catalog.

## Streaming and interruption

Sentence generation runs on a blocking worker. A two-chunk queue limits queued
PCM and passes playback backpressure to inference. Closing or aborting the turn
closes that queue; the Pocket stream stops and joins its inference workers at
the next chunk. The asynchronous server does not wait on native inference.

The Opus packetizer carries incomplete 20 ms frames across model chunks and
pads only once at the sentence boundary. The spoken-text ledger registers a
sentence before audio begins and marks it complete only after generation ends.
Catching up with a partially generated sentence does not count as hearing it all.

## Installation and licenses

The existing voice installer stages all pack files, checks size bounds and
SHA-256 digests, then swaps the directory into place. Cancellation discards the
staging directory. A runtime load failure quarantines the new pack. The installer
reloads engines before reporting success. Old Kokoro model directories are no
longer used; they are not automatically deleted from the owner's data directory.

BoBe's own code remains MIT. Model weights are CC BY 4.0. Alba is attributed to
Alba MacKenna; Azelma/Eponine/Fantine come from the CC BY 4.0 VCTK dataset;
Javert/Marius come from CC0 voice donations. `NOTICE.txt` is installed with every
pack. Preserve it when redistributing weights or preset embeddings.

Cosette (Expresso) and Jean (EARS) are excluded because their recordings are
noncommercial. Do not restore them merely because they appear in a model mirror.

The software license texts added for this migration are in
`third_party/speech-licenses`. The recognition-only adapter's provenance is in
`third_party/sherpa-onnx-sys/README.md`.

## Validation on the development Mac

Live checks exercised the real installer (cancel, retry, concurrent-install
rejection, interrupted-install recovery and engine reload), native audio generation, speed changes and stream
cancellation. The real voice WebSocket negotiated English Pocket, migrated an
old voice ID to Alba, and rejected an uninstalled French pack. Generated speech
was fed back over its PCM capture protocol into the recognition-only STT engine.
This verifies the audio path, not subjective listening quality.

A local release-build sample produced 6.56 seconds of audio in 0.43 seconds,
with the first PCM chunk at 47 ms. This is one local measurement, not a benchmark
against Kokoro or a guarantee of first audible speech latency.

Full provider-backed spoken conversation and physical-device playback still
need a configured provider and device. Linux and Intel macOS runtime behavior
have not been exercised on this development Mac.
