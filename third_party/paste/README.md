# paste compatibility crate

Pocket TTS dependencies still request the archived `paste` crate
(RUSTSEC-2024-0436). This crate re-exports the equivalent macros from the
maintained MIT/Apache-2.0 `pastey` fork. Remove the patch once those
dependencies use `pastey` directly.
