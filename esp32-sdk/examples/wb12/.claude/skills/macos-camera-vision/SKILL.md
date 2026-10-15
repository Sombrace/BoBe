---
name: macos-camera-vision
description: >-
  Use only when explicitly invoked for camera discovery, camera permission,
  bounded frame capture, rectification or Vision OCR through
  `tools/bobe_capture.sh`. Camera presence, permission, mounting and calibration
  are not assumed.
disable-model-invocation: true
compatibility: macOS with Swift/Xcode; live capture needs macOS camera permission.
---

# macOS camera and Vision tool

Read `tools/bobe_capture.sh --help` and the command's help, and always use this
wrapper: it builds and signs the app that holds the camera permission. `list` and
`preflight` only report; `authorize` shows the consent prompt; `frame`,
`rectify` and `ocr` are separate steps. A photo is not a physical display pass
without matching calibration (Calibration in `hil/README.md`).
