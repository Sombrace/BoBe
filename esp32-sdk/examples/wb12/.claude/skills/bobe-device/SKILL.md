---
name: bobe-device
description: Build, flash, identify, or observe the real WB-12 device. Explicit invocation only; never use this skill to choose product work or implementation priorities.
argument-hint: "[build|status|identify|observe|flash] [arguments]"
disable-model-invocation: true
compatibility: ESP-IDF project with a WB-12 connected over USB.
---

# BoBe device operations

Use only for the device operation explicitly requested in `$ARGUMENTS`.

Read the relevant command help before acting. Prefer `tools/idf.sh build` for firmware builds and `tools/bobe_hil.sh device ...` for status, identity, observation, and bounded flashing. Treat flashing as an outward hardware mutation: inspect the image and target first, and do not flash unless the user explicitly requested it.

Do not expand this task into fixture, camera, audio, router, or scenario work. Report unavailable hardware as a blocker rather than substituting simulation.
