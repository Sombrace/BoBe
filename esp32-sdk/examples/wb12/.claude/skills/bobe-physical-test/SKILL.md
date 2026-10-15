---
name: bobe-physical-test
description: Run an explicitly requested WB-12 hardware scenario. Explicit invocation only.
argument-hint: "[scenario-name] [--dry-run|--mock|--live]"
disable-model-invocation: true
compatibility: Project-local uv environment; live runs need the configured lab hardware.
---

# WB-12 physical validation

Use `tools/bobe_hil.sh scenario list` and `scenario show`, then run the scenario
in `$ARGUMENTS`. Read `hil/README.md` only as needed.

Do not widen the request into fixture, router, camera, observer or evidence-tool
work. Use `--dry-run` or `--mock` only when asked; neither proves physical
behavior. A live run that lacks hardware is blocked, not failed or simulated.
