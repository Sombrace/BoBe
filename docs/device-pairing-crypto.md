# Device pairing

A Mac or phone pairs with a daemon by entering a six-digit code. The exchange
is RFC 9807 OPAQUE-3DH from `opaque-ke` 4.0.1, pinned exactly and compiled once
in Rust for the daemon and the Apple apps; Swift never implements OPAQUE.

## Suite

- OPRF and key exchange: Ristretto255, TripleDH
- Hash, KDF and MAC: SHA-512
- Key stretching: Argon2id with the `argon2` crate defaults (19 MiB, two
  passes, one lane) and OPAQUE's fixed zero salt

## C ABI

`PairingCore` owns the OPAQUE states, messages, session key, code validation and
error normalization. `PairingFFI` exposes only the Apple client role
(`PairingFFI/include/bobe_pairing.h`): start and finish login, the
certificate's SPKI SHA-256, and destroying client state and zeroized buffers.
`scripts/tasks/build-pairing-ios.sh` builds the iPhone XCFramework.

- Pointers must be valid for their declared lengths; null is valid only for a
  zero-length input. The SPKI digest output needs 32 writable bytes.
- Handles and buffers are destroyed exactly once through the matching Rust
  export, never a C or Swift allocator.
- A client handle needs exclusive access during finish and destroy. Exports
  contain panics, which does not make invalid pointers or shared handles safe.

## Context binding

The OPAQUE context binds the protocol and suite version, roles, the authority
UUID and epoch, the normalized root origin, the transport (`https` or
`loopback_http`), the SHA-256 of the TLS SubjectPublicKeyInfo the client saw
(empty only for loopback HTTP), the exchange nonces and the scope profile
(`mac_app.v1` or `phone.v1`). Each side refuses any other profile. Enrollment
after OPAQUE is encrypted with a key derived from the session key, because the
self-signed TLS channel is not trusted until key confirmation.

## Lifecycle

- `bobe device pair` creates one invitation that expires after five minutes
  and allows five failed confirmations; begin requests are rate-limited per
  source and per authority.
- A phone invitation also yields a `bobe://pair` link with the code, the
  daemon's origins and its TLS key digest, so the phone pins its first
  connection.
- Enrollment commits the endpoint, its scopes and the credential verifier in
  one transaction, then consumes the invitation. A failed or cancelled
  request releases the code for a retry within its expiry, and an unfinished
  enrollment holds it for at most a minute. If the reply to a committed
  enrollment is lost, the same exchange can fetch it once more within a
  minute.
- The credential is `bobe_v2_<lookup-id>_<256-bit secret>` and only its
  verifier is stored; an owner token may not use that prefix. Re-enrollment
  or revocation revokes earlier credentials and closes their sessions.
