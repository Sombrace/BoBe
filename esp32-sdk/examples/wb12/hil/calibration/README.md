# Physical calibration data

The JSON files in this directory are schemas and examples, not measured
thresholds. Copy real records to the gitignored repository-local path:

```text
private/hil/calibration/camera.json
private/hil/calibration/audio.json
```

Records must be regular non-symlink files. Placeholder zero thresholds are not
recommendations.

Calibration is reference data used by the media and camera steps. An optional
`expires_at` is reported by `bobe-hil doctor` as an `expired` fact. Expired
calibration never blocks a tool; distrust comparisons that use its thresholds.

Camera readiness additionally runs `bobe-capture preflight` against the exact
persistent camera ID. The record binds output dimensions, orientation,
mirroring and normalized ROI.

Audio readiness additionally requires exactly one matching CoreAudio input and
one matching output device. The record binds sample rate and the correlation,
latency, clipping and dropout limits used to judge a capture. A clean host
stream alone is not calibration.

Changing hardware, geometry, gain, lighting, camera format, OS capture behavior
or fixture mounting invalidates the corresponding record.
