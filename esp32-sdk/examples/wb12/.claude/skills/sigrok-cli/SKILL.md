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

Read `tools/sigrok.sh --help` and the command's help; the tool reports its
runtime, decoders, analyzer identity, firmware state, bounds and side effects.
`runtime-status`, `discover`, `doctor`, `inspect` and `convert` do not change
hardware. `probe` and `capture` load `fx2lafw` into the analyzer's RAM, never
its EEPROM; a power cycle restores it. Run `tools/bootstrap_sigrok_cli.sh` when
the runtime is missing or stale.

A compatible driver profile does not prove a clone's input protection. Until a
trustworthy manual or a PCB inspection shows otherwise, connect only 0–3.3 V
logic: never negative, RS-232, mains, automotive, analog or high-energy signals,
and assume neither 1.8 V thresholds nor 5 V tolerance; use level translation or
hardware with a verified rating for anything else. Power the analyzer before
connecting signals, connect ground first, and disconnect signals before removing
analyzer power. Never open or probe a target with this tool; the WB-12 has no
approved internal probe access.
