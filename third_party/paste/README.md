# Maintained token-pasting macros

Pocket's CPU kernels and tokenizer still depend on the archived `paste` crate
(RUSTSEC-2024-0436). This local compatibility crate contains no copy of that
implementation: it re-exports the equivalent macros from the maintained
MIT/Apache-2.0 `pastey` fork, which Pocket already uses elsewhere.

Remove this patch when those upstream dependencies use `pastey` directly.
The adapter is covered by BoBe's MIT license.
