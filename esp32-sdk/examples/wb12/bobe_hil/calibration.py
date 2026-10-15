import json
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from bobe_hil.paths import private_root, regular_file


CALIBRATION_SCHEMA = 1


def calibration_root(root: Path | None = None) -> Path:
    return private_root(root) / "calibration"


def load_calibration(path: Path, description: str) -> dict[str, Any]:
    regular_file(path, description)
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError(f"{description} is invalid JSON") from error
    if not isinstance(value, dict) or value.get("schema_version") != CALIBRATION_SCHEMA:
        raise ValueError(f"{description} schema is invalid")
    return value


def expiry(value: Any) -> str | None:
    if not isinstance(value, str):
        return None
    try:
        parsed = datetime.fromisoformat(value)
    except ValueError:
        return None
    if parsed.tzinfo is None:
        return None
    return parsed.astimezone(UTC).isoformat()


def load_camera_calibration(root: Path | None = None) -> dict[str, Any]:
    value = load_calibration(
        calibration_root(root) / "camera.json",
        "camera calibration",
    )
    roi = value.get("roi")
    thresholds = value.get("thresholds")
    if (
        value.get("target") != "wb12-screen"
        or not isinstance(value.get("camera_id"), str)
        or not value["camera_id"]
        or type(value.get("width")) is not int
        or type(value.get("height")) is not int
        or not 1 <= value["width"] <= 4096
        or not 1 <= value["height"] <= 4096
        or not isinstance(roi, dict)
        or set(roi) != {"x", "y", "width", "height"}
        or any(type(roi[key]) not in {int, float} for key in roi)
        or not 0 <= roi["x"] < 1
        or not 0 <= roi["y"] < 1
        or not 0 < roi["width"] <= 1
        or not 0 < roi["height"] <= 1
        or roi["x"] + roi["width"] > 1
        or roi["y"] + roi["height"] > 1
        or not isinstance(thresholds, dict)
        or set(thresholds) != {
            "max_channel_error",
            "max_mean_absolute_channel_error",
            "max_mismatched_pixel_ratio",
        }
    ):
        raise ValueError("camera calibration values are invalid")
    return {**value, "expires_at": expiry(value.get("expires_at"))}


def load_audio_calibration(root: Path | None = None) -> dict[str, Any]:
    value = load_calibration(
        calibration_root(root) / "audio.json",
        "audio calibration",
    )
    thresholds = value.get("thresholds")
    if (
        value.get("target") != "wb12-audio"
        or not isinstance(value.get("input_device_name"), str)
        or not value["input_device_name"]
        or not isinstance(value.get("output_device_name"), str)
        or not value["output_device_name"]
        or type(value.get("sample_rate")) is not int
        or not 8000 <= value["sample_rate"] <= 96000
        or not isinstance(thresholds, dict)
        or set(thresholds) != {
            "max_clipped_samples",
            "max_dropout_frames_20ms",
            "max_latency_ms",
            "min_normalized_correlation",
        }
    ):
        raise ValueError("audio calibration values are invalid")
    return {**value, "expires_at": expiry(value.get("expires_at"))}


def calibration_status(loader: Any, root: Path | None) -> dict[str, Any]:
    try:
        value = loader(root)
    except (OSError, ValueError) as error:
        return {"configured": False, "blocker": str(error)}
    expires_at = value.get("expires_at")
    expired = (
        expires_at is not None
        and datetime.fromisoformat(expires_at) <= datetime.now(UTC)
    )
    return {
        "configured": True,
        "blocker": None,
        "expires_at": expires_at,
        "expired": expired,
        "calibration": value,
    }


def inspect_calibrations(root: Path | None = None) -> dict[str, Any]:
    return {
        "camera": calibration_status(load_camera_calibration, root),
        "audio": calibration_status(load_audio_calibration, root),
    }
