---
name: bobe-device
description: Build, flash, identify or observe the real WB-12. Explicit invocation only.
argument-hint: "[build|status|identify|observe|flash] [arguments]"
disable-model-invocation: true
compatibility: ESP-IDF project with a WB-12 connected over USB.
---

Run only the operation in `$ARGUMENTS`, after reading its `--help`:
`tools/idf.sh build` or `tools/bobe_hil.sh device status|identify|observe|flash`.
Flash only when asked, after checking the image and the target. Report missing
hardware as a blocker; never simulate it.
