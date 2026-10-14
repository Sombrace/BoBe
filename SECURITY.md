# Security policy

Only the latest release receives security fixes.

## Reporting a vulnerability

Do not open a public issue. Report privately through
[GitHub private vulnerability reporting](https://github.com/Sombrace/BoBe/security/advisories/new)
with a description, reproduction steps or a proof of concept, the impact you
assessed and, optionally, a fix. We acknowledge reports within 48 hours and aim
to provide a fix or mitigation plan within 7 days of confirming one.

## Scope

In scope: the daemon (`BoBeService/`), the Mac and iPhone apps, device pairing
and BodyLink, the ESP32 SDK, MCP integration, and the build, signing, packaging
and update tooling.

Out of scope: vulnerabilities in upstream dependencies (report them upstream),
attacks that need physical access to the machine, and social engineering.

## Design

- The daemon listens on `127.0.0.1` by default. Remote access requires allowed
  hosts, bearer authentication and TLS.
- Every owner-facing HTTP route validates the host. BodyLink listeners use mTLS
  device identity, with both CA validation and an exact enrolled leaf-certificate
  match, or a scoped loopback adapter token; hello JSON alone cannot claim an
  identity.
- The Mac speech adapter's loopback socket uses its own scoped token and never
  receives the owner credential.
- Secrets go through the platform secret store and are held with the `secrecy`
  crate.
- Tools run as the owner's account from BoBe's workspace. This is a single-owner
  boundary, not an operating-system sandbox; `bobe agent tools disable <tool>`
  turns a tool off. Memory and goal changes go through daemon-owned tools.
- MCP commands are checked against a configurable blocklist.
- CORS defaults to localhost origins and is not authentication.
