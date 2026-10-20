# Speech

Speech is recognized and synthesized on the owner's hardware, and raw audio is
not stored. Models download from their publishers on first use and are not
part of BoBe's source or releases. On the Mac, macOS supplies the recognizer
and the voices for languages other than English.

| Surface | Recognition | Synthesis |
|---|---|---|
| Mac app | Apple Speech: `SpeechTranscriber` where macOS supports the language, otherwise `DictationTranscriber` (Greek) | The daemon's Pocket TTS for English, streamed as Opus; the Mac's best system voice for other languages |
| Daemon (headless, physical body) | sherpa-onnx streaming Zipformer, English | Pocket TTS, English |

macOS downloads, stores and updates the speech recognition model for each
language through `AssetInventory`, and BoBe reserves the one it listens with.
BoBe ends a turn when you pause for the chosen pause sensitivity (Tight
400 ms, Balanced 800 ms, Patient 1.5 s), then finalizes that utterance before
sending it. When the English model is still downloading, the daemon's English
recognizer hears you instead; other languages wait for the download. Languages
other than English are spoken with the best installed system voice: Premium,
then Enhanced, then the default. Owners can add better voices in System
Settings › Accessibility. Pocket TTS needs about 170 MB. The daemon runs Pocket
TTS natively through `ptts` with no Python or model server, and
`BoBeService/src/voice/install_artifacts.rs` pins every model file's SHA-256.
Building the daemon needs a C++ compiler and libclang. Try a voice without a
provider with `bobe voice preview` ([command line](cli.md#working-with-bobe)).

## Models and licenses

Each license applies to its model, not to BoBe's code.

| Model | Used for | License | Source |
|---|---|---|---|
| Pocket TTS, English b6369a24 generation, Q8 GGUF | Daemon voice | CC BY 4.0, Kyutai | Third-party conversion [`davencyw/pocket-tts-ptts`](https://huggingface.co/davencyw/pocket-tts-ptts) at a pinned revision, of [`kyutai/pocket-tts-without-voice-cloning`](https://huggingface.co/kyutai/pocket-tts-without-voice-cloning) |
| Voices Alba; Azelma, Eponine, Fantine; Javert, Marius | Daemon voice presets | CC BY 4.0 (Alba MacKenna); CC BY 4.0 (CSTR VCTK corpus); CC0 (Kyutai voice donations) | [`kyutai/tts-voices`](https://huggingface.co/kyutai/tts-voices) |
| Streaming Zipformer English 20M | Daemon recognition | Apache-2.0 | [sherpa-onnx ASR models](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models) |

The Pocket TTS pack installs a `NOTICE.txt` with its attribution; keep it when
redistributing the weights or voice embeddings. Cosette (Expresso) and Jean
(EARS) are excluded because their recordings are licensed for noncommercial use
only; do not restore them from a model mirror. Apple's speech recognition
models and system voices are part of macOS and are covered by its license;
BoBe neither ships nor redistributes them.
