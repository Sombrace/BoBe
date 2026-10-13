---
name: sigrok-cli
description: >-
  Use only when explicitly invoked for project-local FX2 logic-analyzer discovery,
  qualification, capture, offline session inspection/conversion, or electrical
  judgments through `tools/sigrok.sh`. Never use this skill to choose firmware
  roadmap work. Analyzer presence, firmware state, wiring, and calibration are
  not assumed.
disable-model-invocation: true
compatibility: macOS Apple silicon; runtime inputs are pinned by tools/sigrok-cli.lock.
---

# Project-local sigrok tool

Read `tools/sigrok.sh --help` and the selected subcommand help. Runtime,
identity, firmware state, capture bounds, and operation side effects are
reported by the tool.

Read `references/runtime.md` when the native runtime is missing or stale,
`references/electrical.md` before connecting an unverified clone, and
`references/tn077-te001.md` only for the observed TN077/TE001.
