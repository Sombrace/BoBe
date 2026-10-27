# Recognition-only sherpa-onnx bindings

`src/` and `LICENSE` are unchanged from the crates.io `sherpa-onnx-sys` 2.0.1
package by the [sherpa-onnx project](https://github.com/k2-fsa/sherpa-onnx).
BoBe replaces the manifest and build script so the build links upstream's
`static-no-tts-lib` archives (no eSpeak NG, Piper or speech synthesis),
verified against pinned SHA-256 digests from the v2.0.1 release. Supported
targets are macOS and Linux x86_64; others fail. Updating the version means
updating the FFI declarations and archive pins together.
