# Speech

Speech recognition and synthesis run on the owner's hardware. Raw audio is not
stored. Models are downloaded from their publishers on first use and are not part
of BoBe's source or releases.

## Engines

| Surface | Recognition | Synthesis |
|---|---|---|
| Mac app | Parakeet Realtime EOU for English; Nemotron 3.5 multilingual for other languages; Silero voice activity detection | The daemon's Pocket TTS, streamed as Opus. Supertonic-3 in the app is an experimental option for other languages |
| Daemon (headless, physical body) | sherpa-onnx streaming Zipformer, English | Pocket TTS, English |

English needs about 600 MB: the Pocket TTS pack (about 170 MB) and the Mac's
English listener (about 450 MB). Another language adds a Nemotron pack of about
0.7 GB.

## Models and licenses

Each license applies to its model, not to BoBe's code.

| Model | Used for | License | Source |
|---|---|---|---|
| Pocket TTS, English b6369a24 generation, Q8 GGUF | Daemon voice | CC BY 4.0, Kyutai | Third-party conversion [`davencyw/pocket-tts-ptts`](https://huggingface.co/davencyw/pocket-tts-ptts) at a pinned revision, of [`kyutai/pocket-tts-without-voice-cloning`](https://huggingface.co/kyutai/pocket-tts-without-voice-cloning) |
| Voices Alba; Azelma, Eponine, Fantine; Javert, Marius | Daemon voice presets | CC BY 4.0 (Alba MacKenna); CC BY 4.0 (CSTR VCTK corpus); CC0 (Kyutai voice donations) | [`kyutai/tts-voices`](https://huggingface.co/kyutai/tts-voices) |
| Parakeet Realtime EOU 120M, Core ML | Mac English recognition | [NVIDIA Open Model License](https://www.nvidia.com/en-us/agreements/enterprise-software/nvidia-open-model-license/) | [`FluidInference/parakeet-realtime-eou-120m-coreml`](https://huggingface.co/FluidInference/parakeet-realtime-eou-120m-coreml) |
| Nemotron 3.5 ASR Streaming Multilingual 0.6B, Core ML | Mac recognition, other languages | [OpenMDW-1.1](https://openmdw.ai/license/1-1/) | [`FluidInference/Nemotron-3.5-ASR-Streaming-Multilingual-0.6b-CoreML`](https://huggingface.co/FluidInference/Nemotron-3.5-ASR-Streaming-Multilingual-0.6b-CoreML) |
| Silero VAD, Core ML | Mac voice activity detection | MIT | [`FluidInference/silero-vad-coreml`](https://huggingface.co/FluidInference/silero-vad-coreml) |
| Supertonic-3, Core ML | Experimental Mac voice | OpenRAIL-M, with use restrictions | [`FluidInference/supertonic-3-coreml`](https://huggingface.co/FluidInference/supertonic-3-coreml), from [`Supertone/supertonic-3`](https://huggingface.co/Supertone/supertonic-3) |
| Streaming Zipformer English 20M | Daemon recognition | Apache-2.0 | [sherpa-onnx ASR models](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models) |

The Pocket TTS pack installs a `NOTICE.txt` with its attribution; keep it when
redistributing the weights or voice embeddings. Cosette (Expresso) and Jean (EARS)
are excluded because their recordings are licensed for noncommercial use only; do
not restore them from a model mirror. Redistributing Parakeet requires NVIDIA's
notice, "Licensed by NVIDIA Corporation under the NVIDIA Open Model License", and
a copy of that license. Supertonic-3's license forbids the uses listed in its
OpenRAIL-M restrictions.

## Pocket TTS

The daemon runs `ptts` 0.4.0-rc.1 (MIT or Apache-2.0) natively on the CPU with
Q8 weights. No Python, model server or voice cloning is involved. The pack is
`pocket-tts-en-b6369a24-q8` with six presets; Alba is the default and earlier
Kokoro voice selections resolve to it. `BoBeService/src/voice/install_artifacts.rs`
pins the conversion revision and every file's SHA-256. Other Pocket languages need
an independently pinned model, tokenizer and voice pack and an updated capability
catalog; unsupported language and engine combinations fail during negotiation.

Speed from 0.5× to 2× uses Signalsmith Stretch (MIT) with pitch preserved.
Building it needs a C++ compiler and libclang (Xcode on macOS; `clang` and
`libclang-dev` on Linux).

Recognition uses sherpa-onnx 1.13.8 from its `static-no-tts-lib` archives, which
contain no eSpeak NG or Piper. `third_party/sherpa-onnx-sys/README.md` records the
adaptation. Apple Silicon and Intel macOS and Linux x86_64 are supported; other
targets fail the build.

### Streaming and interruption

Generation runs on a blocking worker. A two-chunk queue bounds pending PCM and
passes playback backpressure to inference. Closing or aborting a turn closes the
queue, and the stream stops at the next chunk. The Opus packetizer carries partial
20 ms frames across chunks and pads once per sentence. A sentence is registered
before its audio starts and marked complete only after generation ends, so a
partially generated sentence never counts as heard.

Filler and reply streams keep separate Opus decoder state on the Mac and the body,
and reply decoding resets at each turn. Output sends whole phrases, so filler
cannot interleave with a reply; a filler is skipped, not queued, while another
phrase is in flight.

### Installation

The installer stages every file, checks size bounds and SHA-256 digests, and swaps
the directory into place; cancellation discards the staging directory. A pack that
fails to load is quarantined, and engines reload before success is reported.
Status separates `installed` (files present) from `ready` (engine loaded). One
reinstall repairs an unusable pack. Old Kokoro directories are no longer used and
are not deleted automatically.

## Preview speech without a provider account

With a local daemon running (`bobe init`, then `bobe serve`), install the English
voice and preview text. Neither step needs a model provider or speech recognition:

```sh
bobe voice-models install --kind tts
bobe voice preview "Hello. This is BoBe." --output /tmp/bobe-preview.wav --play --json
```

`--voice marius` selects a preset, `--speed 0.5` to `--speed 2.0` sets tempo, and
`--cancel-after 100ms` cancels after the first audio packet. The output path must
be new. A cancelled preview leaves a valid WAV of the audio received; a failed one
leaves nothing. Omit `--play` to export only; playback uses `afplay` on macOS and
`pw-play` or `aplay` on Linux, and is acknowledged only after the player exits
successfully. The JSON report includes the voice, captions, frame count, audio
duration and first-packet time.

Preview uses the authenticated `/voice/preview` WebSocket with `bobe.voice.v2`,
accepts at most 8,000 bytes of text, and shares the conversation's sentence
cleaner, engine, worker and packetizer. It respects disabled voice and voice-session
exclusion, and writes no conversation turns, memories or goals.
