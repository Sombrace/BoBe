# Device pairing

A Mac or phone pairs with a daemon by entering a six-digit code. The exchange
is RFC 9807 OPAQUE-3DH from `opaque-ke` 4.0.1, pinned exactly and compiled
once in Rust for the daemon and the Apple apps. OPAQUE is used because no
maintained SPAKE2+ implementation spans Rust, macOS and iOS.

## Suite

- OPRF and key exchange: Ristretto255, TripleDH
- Hash, KDF and MAC: SHA-512
- Key stretching: Argon2id with the `argon2` crate defaults (19 MiB, two
  passes, one lane) and OPAQUE's fixed zero salt

## Boundary

`PairingCore` owns the client and server OPAQUE states, wire messages, the
session key, code validation and error normalization. `PairingFFI` exposes
only the Apple client role through a static C ABI: start login, finish login,
destroy state, and zeroize and free buffers. Swift never implements OPAQUE.

ABI rules:

- Pointers must be valid for their declared lengths; null is valid only for a
  zero-length input. The SPKI digest output needs 32 writable bytes.
- Handles and buffers are destroyed exactly once, through the matching Rust
  export, never a C or Swift allocator.
- A client handle needs exclusive access during finish and destroy.
- Exports contain panics, but that does not make invalid pointers or shared
  handles safe.

The Mac links the macOS static library from `PairingFFI/target/release`.
`scripts/tasks/build-pairing-ios.sh` builds
`build/pairing-ios/BoBePairingFFI.xcframework` with device and simulator
slices for the iPhone.

## Context binding

The OPAQUE context binds the protocol and suite version, roles, the authority
UUID and epoch, the normalized root origin, the transport (`https` or
`loopback_http`), the SHA-256 of the TLS SubjectPublicKeyInfo the client saw
(empty only for loopback HTTP), the exchange nonces and the scope profile
(`mac_app.v1` or `phone.v1`). The daemon refuses an exchange whose profile does
not match the invitation, and each client refuses any other profile.

Enrollment after OPAQUE is encrypted with a key derived from the session key,
because the self-signed TLS channel is not trusted until key confirmation.

## Lifecycle

- `bobe device pair` creates one invitation that expires after five minutes
  and allows five failed confirmations. Begin requests are rate-limited per
  source and per authority.
- A phone invitation also yields a `bobe://pair` link carrying the code, the
  daemon's origins and its TLS key digest, so the phone pins its first
  connection.
- Enrollment commits the endpoint, its scopes, the credential verifier and
  the audit event in one transaction, then consumes the invitation. A failure
  releases the invitation for a retry within its expiry.
- The credential is `bobe_v2_<lookup-id>_<256-bit secret>`; only its verifier
  is stored. Re-enrollment or revocation revokes earlier credentials and
  closes their live sessions.
