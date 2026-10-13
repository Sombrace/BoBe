import json
import tempfile
import unittest
from datetime import UTC, datetime, timedelta
from pathlib import Path

from bobe_hil import calibration


CAMERA = {
    "schema_version": 1,
    "target": "wb12-screen",
    "calibration_id": "0" * 32,
    "camera_id": "camera-uid",
    "fixture_id": "optical-rig",
    "width": 280,
    "height": 240,
    "orientation_degrees": 0,
    "mirrored": False,
    "roi": {"x": 0.1, "y": 0.1, "width": 0.5, "height": 0.5},
    "thresholds": {
        "max_channel_error": 24,
        "max_mean_absolute_channel_error": 4.0,
        "max_mismatched_pixel_ratio": 0.02,
    },
}
AUDIO = {
    "schema_version": 1,
    "target": "wb12-audio",
    "calibration_id": "0" * 32,
    "fixture_id": "acoustic-rig",
    "input_device_name": "Fixture Input",
    "output_device_name": "Fixture Output",
    "sample_rate": 48000,
    "thresholds": {
        "max_clipped_samples": 0,
        "max_dropout_frames_20ms": 1,
        "max_latency_ms": 250.0,
        "min_normalized_correlation": 0.5,
    },
}


def write(root: Path, name: str, value: dict[str, object]) -> Path:
    calibration_root = root / "calibration"
    calibration_root.mkdir(exist_ok=True)
    path = calibration_root / name
    path.write_text(json.dumps(value), encoding="utf-8")
    return path


class CalibrationTests(unittest.TestCase):
    def test_calibration_is_data_and_needs_no_ownership(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = write(root, "camera.json", CAMERA)
            path.chmod(0o666)
            value = calibration.load_camera_calibration(root)
            self.assertEqual(value["camera_id"], "camera-uid")
            self.assertEqual(value["roi"]["width"], 0.5)

    def test_expired_calibration_is_reported_not_enforced(self) -> None:
        stale = (datetime.now(UTC) - timedelta(days=1)).isoformat()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, "camera.json", {**CAMERA, "expires_at": stale})
            write(root, "audio.json", {**AUDIO, "expires_at": stale})
            status = calibration.inspect_calibrations(root)
        self.assertTrue(status["camera"]["configured"])
        self.assertTrue(status["camera"]["expired"])
        self.assertIsNone(status["camera"]["blocker"])
        self.assertTrue(status["audio"]["configured"])

    def test_qualified_false_no_longer_blocks_use(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(root, "audio.json", {**AUDIO, "qualified": False})
            status = calibration.inspect_calibrations(root)
        self.assertTrue(status["audio"]["configured"])
        self.assertIsNone(status["audio"]["blocker"])

    def test_invalid_region_of_interest_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            write(
                root,
                "camera.json",
                {**CAMERA, "roi": {"x": 0.8, "y": 0.0, "width": 0.5, "height": 0.5}},
            )
            with self.assertRaises(ValueError):
                calibration.load_camera_calibration(root)

    def test_missing_calibration_reports_absence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            status = calibration.inspect_calibrations(Path(directory))
        self.assertFalse(status["camera"]["configured"])
        self.assertIn("unavailable", status["camera"]["blocker"])

    def test_repository_examples_parse_as_calibration_data(self) -> None:
        root = Path(__file__).resolve().parent.parent / "hil" / "calibration"
        with tempfile.TemporaryDirectory() as directory:
            private = Path(directory)
            write(
                private,
                "camera.json",
                json.loads((root / "camera.example.json").read_text()),
            )
            write(
                private,
                "audio.json",
                json.loads((root / "audio.example.json").read_text()),
            )
            status = calibration.inspect_calibrations(private)
        self.assertTrue(status["camera"]["configured"])
        self.assertTrue(status["audio"]["configured"])


if __name__ == "__main__":
    unittest.main()
