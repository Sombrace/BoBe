---
name: bobe-physical-test
description: Run an explicitly requested BoBe hardware scenario or physical validation. Explicit invocation only; never use this skill to set the firmware roadmap.
argument-hint: "[scenario-name] [--dry-run|--mock|--live]"
disable-model-invocation: true
compatibility: Project-local uv environment; live runs may require configured external hardware.
---

# BoBe physical validation

Use `tools/bobe_hil.sh scenario list`, `scenario show`, and then the exact scenario requested in `$ARGUMENTS`. Read `hil/README.md` only as needed for that operation.

HIL is a side path for validating already-chosen product work. Do not propose fixture, router, camera, observer, or evidence-system development as product milestones. Never turn a firmware implementation request into HIL work unless the user explicitly asks for physical validation.

Use `--dry-run` or `--mock` only when requested; neither proves physical behavior. A live run that lacks required hardware is blocked, not failed and not simulated.
