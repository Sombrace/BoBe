---
name: macos-camera-vision
description: >-
  Use only when explicitly invoked for camera discovery, TCC preflight or
  authorization, bounded frame capture, rectangle rectification, or Vision OCR
  through `tools/bobe_capture.sh`. Never use this skill to choose firmware
  roadmap work. Camera presence, permission, mounting, and calibration are not
  assumed.
disable-model-invocation: true
compatibility: macOS with Swift/Xcode; live capture uses macOS camera authorization.
---

# macOS camera and Vision tool

Read `tools/bobe_capture.sh --help` and the selected command help. The wrapper
builds and signs the app bundle whose identifier owns macOS TCC permission.

`list` and `preflight` report current camera/setup state. `authorize` presents
the macOS consent UI. Frame capture, rectification, and OCR are separate
operations. WB-12 calibration data lives under `hil/calibration/`.
