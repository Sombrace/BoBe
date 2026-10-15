---
name: bobe-device
description: Build, flash, identify or observe the real WB-12. Explicit invocation only.
argument-hint: "[build|status|identify|observe|flash] [arguments]"
disable-model-invocation: true
compatibility: ESP-IDF project with a WB-12 connected over USB.
---

# WB-12 device operations

Run only the operation in `$ARGUMENTS`, after reading its `--help`.

- Build: `tools/idf.sh build`.
- Status, identity, observation and flashing: `tools/bobe_hil.sh device ...`.

Flashing changes hardware: check the image and the target first, and flash only
when asked to. Report missing hardware as a blocker; never simulate it.
