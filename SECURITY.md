# Security Policy

## Supported Versions

| Version | Supported |
|---------|-----------|
| Latest release | ✅ |
| Older releases | ❌ |

Only the latest release receives security updates.

## Reporting a Vulnerability

**Please do not open a public issue for security vulnerabilities.**

Instead, report them privately via **[GitHub Security Advisories](https://github.com/Sombrace/BoBe/security/advisories/new)**.

Include:

- A description of the vulnerability
- Steps to reproduce or a proof-of-concept
- The impact you've assessed
- Any suggested fix (optional)

We will acknowledge receipt within 48 hours and aim to provide a fix or mitigation plan within 7 days for confirmed vulnerabilities.

## Scope

The following are in scope:

- **Rust backend** (`BoBeService/src/`) — API handlers, LLM orchestration, tool execution, file access, config/secrets handling
- **Swift frontend** (`BoBeMacUI/`) — backend communication, credential handling
- **Build and release tooling** — code signing, notarization, packaging, and update distribution
- **MCP server integration** — command validation, environment handling

The following are out of scope:

- Vulnerabilities in upstream dependencies (report those to the upstream project)
- Issues requiring physical access to the machine
- Social engineering attacks

## Security Design

BoBe is designed with the following security properties:

- The daemon binds to `127.0.0.1` by default; deliberate remote exposure requires allowed hosts, bearer authentication, and TLS
- Host validation middleware on every owner-facing HTTP route; the separate
  BodyLink listeners use mTLS device identity or a scoped loopback adapter token
- Secrets are stored through the platform secret-store abstraction and handled in-memory with the `secrecy` crate
- Tools run with the owner's user account and start in BoBe's workspace. This is a single-owner boundary, not an operating-system sandbox. The owner can disable a tool with `bobe agent tools disable <tool>`. Memory and goal changes go through daemon-owned tools
- MCP commands are validated against a configurable blocklist
- CORS defaults to localhost origins and is not treated as authentication
- The optional BodyLink listener requires both CA validation and an exact
  enrolled client-certificate leaf match; device identity cannot be supplied
  by hello JSON alone
- The loopback Swift body speech-adapter socket uses a separate scoped bearer
  token and never receives the broad owner-facing daemon credential
