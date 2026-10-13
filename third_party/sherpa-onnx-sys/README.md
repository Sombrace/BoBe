# Recognition-only sherpa-onnx bindings

`src/` and `LICENSE` are unchanged files from the crates.io
`sherpa-onnx-sys` 1.13.8 package, published by the
[sherpa-onnx project](https://github.com/k2-fsa/sherpa-onnx).

BoBe replaces the package manifest and build script. The build uses upstream's
`static-no-tts-lib` archives, retaining the official Rust API for speech
recognition while excluding eSpeak NG, Piper phonemization and speech synthesis.
The archive SHA-256 values come from the upstream v1.13.8 GitHub release asset
digests and are verified before extraction. An external archive directory is
permitted only when its archive matches the same pinned digest.

Supported daemon targets are Apple Silicon macOS, Intel macOS and Linux x86_64.
An unsupported target fails explicitly instead of selecting a build containing
speech synthesis. This local adaptation is maintained with the daemon; an
upstream version update must update the FFI declarations and archive pins
together.
