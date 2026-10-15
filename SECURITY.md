# Security policy

Only the latest release receives security fixes.

## Reporting a vulnerability

Do not open a public issue. Report privately through
[GitHub private vulnerability reporting](https://github.com/Sombrace/BoBe/security/advisories/new)
with a description, reproduction steps or a proof of concept, and the impact.
We acknowledge reports within 48 hours and aim to provide a fix or mitigation
plan within 7 days of confirming one.

## Scope

In scope: the daemon, the Mac and iPhone apps, pairing and BodyLink, the ESP32
SDK, MCP integration, and the build, signing, packaging and update tooling.

Out of scope: vulnerabilities in upstream dependencies (report them upstream),
attacks that need physical access to the machine, and social engineering.

## Boundaries

- The daemon listens on loopback by default; remote access needs allowed
  hosts, per-device credentials and TLS. Bodies use mutual TLS with enrolled
  certificates.
- Tools run as the owner's account from BoBe's workspace. This is a
  single-owner boundary, not an operating-system sandbox.
- MCP commands are checked against a blocklist.
