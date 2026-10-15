---
name: sigrok-cli
description: >-
  Use only when explicitly invoked to discover, qualify or capture with a
  project-local FX2 logic analyzer, to inspect or convert sigrok sessions, or to
  judge whether a signal is electrically safe to connect, through
  `tools/sigrok.sh`. Analyzer presence, firmware state, wiring and calibration
  are not assumed.
disable-model-invocation: true
compatibility: macOS on Apple silicon; runtime pinned by tools/sigrok-cli.lock.
---

# Project-local sigrok

Read `tools/sigrok.sh --help` and the command's help; the tool reports its
runtime, decoders, analyzer identity, firmware state, capture bounds and side
effects. `runtime-status`, `discover`, `doctor`, `inspect` and `convert` do not
change hardware. `probe` and `capture` load `fx2lafw` into the analyzer's RAM
(never its EEPROM; a power cycle restores it). If the runtime is missing or
stale, run `tools/bootstrap_sigrok_cli.sh`.

## Electrical limits

A compatible driver profile does not prove a clone's input protection. Until a
trustworthy manual or a PCB inspection shows otherwise:

- use only 0–3.3 V logic; never negative, RS-232, mains, automotive, analog or
  high-energy signals, and do not assume 1.8 V thresholds or 5 V tolerance;
- power the analyzer before connecting signals, connect ground first, and
  disconnect signals before removing analyzer power;
- use level translation or hardware with a verified rating otherwise.

This tool never authorizes opening or probing a target. The WB-12 has no
approved internal probe access.
