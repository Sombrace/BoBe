# Speech

Speech is recognized and synthesized on the owner's hardware, and raw audio is
not stored. Models download from their publishers on first use and are not
part of BoBe's source or releases.

| Surface | Recognition | Synthesis |
|---|---|---|
| Mac app | Parakeet Realtime EOU for English, Nemotron 3.5 for other languages, Silero VAD | The daemon's Pocket TTS, streamed as Opus; Supertonic-3 in the app is an experimental option for other languages |
| Daemon (headless, physical body) | sherpa-onnx streaming Zipformer, English | Pocket TTS, English |

English needs about 600 MB; each other language adds about 0.7 GB. The daemon
runs Pocket TTS natively through `ptts` with no Python or model server, and
`BoBeService/src/voice/install_artifacts.rs` pins every model file's SHA-256.
Building the daemon needs a C++ compiler and libclang. Try a voice without a
provider with `bobe voice preview` ([command line](cli.md#working-with-bobe)).

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
redistributing the weights or voice embeddings. Cosette (Expresso) and Jean
(EARS) are excluded because their recordings are licensed for noncommercial use
only; do not restore them from a model mirror. Redistributing Parakeet requires
NVIDIA's notice, "Licensed by NVIDIA Corporation under the NVIDIA Open Model
License", and a copy of that license. Supertonic-3's license forbids the uses
listed in its OpenRAIL-M restrictions.
