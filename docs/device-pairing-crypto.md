# Device Pairing Cryptography Decision

## Decision

BoBe device pairing uses the RFC 9807 OPAQUE-3DH implementation from
`opaque-ke` 4.0.1, pinned exactly and compiled once for the Rust daemon and
Apple clients. The Apple application consumes only a narrow static C ABI; it
does not implement OPAQUE or elliptic-curve operations in Swift.

The suite is fixed to:

- OPRF group: Ristretto255
- Key exchange: TripleDH with Ristretto255
- Hash/KDF/MAC family: SHA-512 through the upstream suite
- Key stretching: Argon2id through `opaque-ke`'s Argon2 KSF

The KSF uses the `argon2` crate's defaults: version 0x13, 19 MiB of memory, two
passes and one lane, with the fixed zero salt OPAQUE allows because its input is
already the OPRF output. The daemon runs it once when it creates an invitation;
the phone or Mac runs it once per pairing attempt.

The six-digit code is a short-lived, one-use pairing secret, not a durable
password or operational credential. Pairing policy—not the PAKE alone—must
enforce five-minute expiry, one active invitation, five attempts, source and
authority rate limits, no secret logging, and deletion on success, expiry, or
lockout.

## Why not SPAKE2+

RFC 9383 SPAKE2+ is the better conceptual fit and Matter establishes strong
short-code commissioning precedent. No production-ready implementation with a
supported public API currently spans Rust, macOS, and iOS, however. The viable
options would require BoBe to own an unaudited wrapper around BoringSSL's
internal C++ API or adopt a very young unaudited Rust crate. Apple exposes no
generic SPAKE2+ API.

`opaque-ke` has a final-RFC implementation, official vectors, active
maintenance, no unsafe Rust in the upstream library, zeroization support,
`no_std` support, and historical NCC Group review lineage. The audit predates
4.x, so this remains a dependency risk requiring focused review.

## Boundary

`PairingCore` owns:

- server-side temporary registration generated locally from the displayed code;
- client and server OPAQUE login states;
- serialized wire messages;
- the shared session key;
- code format validation;
- protocol error normalization.

`PairingFFI` exposes only the Apple client role:

- start client login and return the first message;
- finish client login and return finalization plus session key;
- destroy opaque client state;
- zeroize and destroy Rust-owned buffers.

All exported functions are panic-contained. C callers never allocate or free
Rust state directly, and Swift never sees generic OPAQUE structures.

The Mac app links the macOS static library from `PairingFFI/target/release`.
For the iPhone, `scripts/tasks/build-pairing-ios.sh` builds the same library for
`aarch64-apple-ios` and `aarch64-apple-ios-sim` with iOS 26 as the minimum and
packages `build/pairing-ios/BoBePairingFFI.xcframework`. It has an `ios-arm64`
and an `ios-arm64-simulator` slice, each carrying `bobe_pairing.h` and a
generated module map for a `BoBePairingFFI` module. Either slice links into an
iOS executable with nothing beyond the system library.

The C ABI requires valid pointers for their declared lengths, writable output
storage, and exclusive access to a client handle during finish/destroy. A null
input pointer is valid only for a zero-length input. Handles and buffers must be
destroyed exactly once through the matching Rust export, never through a C/Swift
allocator; use-after-destroy and concurrent handle mutation are invalid. Buffer
destruction zeroizes its contents and clears its pointer/length. The SPKI digest
output requires 32 writable bytes. Panic containment does not make invalid
pointers or concurrent ownership safe.

## Required context binding

The application-supplied OPAQUE context must be a canonical versioned encoding
that binds at least:

- BoBe pairing protocol and ciphersuite version;
- client/server roles;
- runtime authority UUID and epoch;
- canonical root server origin with normalized default ports and no credentials,
  query, fragment, or non-root path;
- transport marker: `https` or `loopback_http`;
- SHA-256 of the TLS SubjectPublicKeyInfo observed by the client, empty only for
  loopback HTTP;
- invitation/exchange nonces;
- requested endpoint kind and default scope-profile identifier.

The scope-profile identifier names the endpoint kind as well as its scopes:
`mac_app.v1` for a Mac and `phone.v1` for a phone. The daemon takes it from the
active invitation, which the owner created for one kind, and refuses an
exchange whose profile no longer matches that invitation. The Mac refuses a
transcript that names any other profile; the phone must refuse one that is not
`phone.v1`.

OPAQUE already binds its protocol messages and identities. BoBe's context binds
the exchange to the expected runtime and initially observed TLS identity. The
post-OPAQUE enrollment request and credential response must use an AEAD key
derived from the session key because the self-signed TLS channel is not trusted
until key confirmation succeeds.

## Pairing lifecycle

`bobe device pair` creates one six-digit invitation for five minutes. For a
phone it also returns a `bobe://pair` link with the code, the daemon's origins
and the SHA-256 of its TLS SubjectPublicKeyInfo, so the phone pins its first
TLS connection before the exchange; the transcript still binds the SPKI the
phone observed. Each invitation permits five failed confirmations. Begin
requests are additionally bounded by fixed per-source and authority-wide time
windows. Exchanges from the same source coexist; a new attempt does not evict
an exchange already in flight.

The confirmed exchange reserves the invitation while the daemon decrypts and
validates enrollment. Endpoint metadata, the profile's scopes (every owner
scope for a Mac, the narrower phone set for a phone), the verifier-only bearer
generation, supersession records, and the security audit event commit in one
SQLite transaction. A pairing that names an active endpoint enrolled as another
kind is refused before anything changes. Only then is the invitation consumed.
Any validation, database, timeout, or cancellation failure releases the
reservation so the same valid code can be retried until its normal expiry or
attempt budget is exhausted.

The issued bearer is `bobe_v2_<lookup-id>_<256-bit-secret>`. Only its verifier is
stored. Re-enrollment revokes prior generations and immediately cancels their
live SSE and voice leases. Endpoint revocation atomically revokes credentials and
scope grants, records the audit event, and cancels all matching long-lived
sessions.

## Remaining release gates

- Complete a pairing from a real phone against a networked daemon.
- Run upstream RFC 9807 vectors through the BoBe wrapper, not only upstream
  tests.
- Freeze BoBe application vectors for its exact Argon2 parameters, identifiers,
  context encoding, and wire serialization.
- Test malformed messages, replay, state reuse, panic containment, concurrent
  sessions, and allocation failure behavior at the ABI boundary.
- Measure the Argon2id step on the slowest phone iOS 26 supports, an A13
  iPhone 11 or second-generation iPhone SE, before freezing the pairing
  protocol version. The simulator measures the Mac's cores, not a phone's.
- Produce dependency provenance, SBOM, license notices, and export-compliance
  review for release artifacts.
