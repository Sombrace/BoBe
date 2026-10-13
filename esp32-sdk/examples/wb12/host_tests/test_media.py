import tempfile
import unittest
from pathlib import Path
from unittest import mock

import numpy as np
from PIL import Image
import sounddevice

from bobe_hil import cli, media


class MediaTests(unittest.TestCase):
    def test_rejects_unbounded_images_and_invalid_wav_rate(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            expected = root / "expected.png"
            actual = root / "actual.png"
            Image.new("RGB", (4097, 1)).save(expected)
            Image.new("RGB", (1, 1)).save(actual)
            with self.assertRaisesRegex(ValueError, "dimensions exceed"):
                media.image_metrics(expected, actual)

        class InvalidRateWave:
            def __enter__(self) -> "InvalidRateWave":
                return self

            def __exit__(self, *_args: object) -> None:
                return None

            @staticmethod
            def getnchannels() -> int:
                return 1

            @staticmethod
            def getsampwidth() -> int:
                return 2

            @staticmethod
            def getframerate() -> int:
                return 0

            @staticmethod
            def getnframes() -> int:
                return 1

        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "invalid.wav"
            source.write_bytes(b"not-empty")
            with mock.patch.object(
                media.wave,
                "open",
                return_value=InvalidRateWave(),
            ):
                with self.assertRaisesRegex(ValueError, "sample rate"):
                    media.read_wav(source)

    def test_audio_challenge_and_latency_metrics(self) -> None:
        reference = media.audio_challenge(
            "12" * 16,
            sample_rate=16000,
            duration_seconds=0.5,
        )
        capture = np.concatenate(
            (
                np.zeros(320, dtype=np.int16),
                reference,
                np.zeros(160, dtype=np.int16),
            )
        )
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            reference_path = root / "reference.wav"
            capture_path = root / "capture.wav"
            media.write_wav(reference_path, reference, 16000)
            media.write_wav(capture_path, capture, 16000)
            metrics = media.audio_metrics(reference_path, capture_path)
        self.assertEqual(metrics["latency_samples"], 320)
        self.assertAlmostEqual(metrics["latency_ms"], 20.0)
        self.assertGreater(metrics["normalized_correlation"], 0.999)
        self.assertEqual(metrics["clipped_samples"], 0)

    def test_audio_challenge_is_nonce_bound(self) -> None:
        first = media.audio_challenge("ab" * 16)
        self.assertTrue(np.array_equal(first, media.audio_challenge("ab" * 16)))
        self.assertFalse(np.array_equal(first, media.audio_challenge("cd" * 16)))

    def test_image_metrics_report_exact_and_changed_pixels(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            expected = root / "expected.png"
            actual = root / "actual.png"
            changed = root / "changed.png"
            Image.new("RGB", (8, 6), (10, 20, 30)).save(expected)
            Image.new("RGB", (8, 6), (10, 20, 30)).save(actual)
            image = Image.new("RGB", (8, 6), (10, 20, 30))
            image.putpixel((2, 3), (11, 22, 33))
            image.save(changed)
            exact = media.image_metrics(expected, actual)
            difference = media.image_metrics(expected, changed)
        self.assertEqual(exact["mismatched_pixels"], 0)
        self.assertEqual(exact["max_channel_error"], 0)
        self.assertEqual(difference["mismatched_pixels"], 1)
        self.assertEqual(difference["max_channel_error"], 3)

    def test_image_metrics_reject_dimension_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            expected = root / "expected.png"
            actual = root / "actual.png"
            Image.new("RGB", (8, 6)).save(expected)
            Image.new("RGB", (7, 6)).save(actual)
            with self.assertRaisesRegex(ValueError, "dimensions differ"):
                media.image_metrics(expected, actual)

    def test_rgb565_conversion_uses_canonical_big_endian_words(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "pixels.rgb565"
            output = root / "pixels.png"
            source.write_bytes(bytes.fromhex("f80007e0001f"))
            media.rgb565_to_png(source, output, width=3, height=1)
            with Image.open(output) as image:
                self.assertEqual(
                    [
                        image.convert("RGB").getpixel((x, 0))
                        for x in range(3)
                    ],
                    [(255, 0, 0), (0, 255, 0), (0, 0, 255)],
                )

    def test_rgb565_cli_requires_explicit_dimensions(self) -> None:
        parser = cli.build_parser()
        with self.assertRaises(SystemExit):
            parser.parse_args(
                [
                    "media",
                    "rgb565-to-png",
                    "frame.rgb565",
                    "frame.png",
                ]
            )

    def test_play_record_uses_exact_devices_and_bounded_output(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stimulus = root / "stimulus.wav"
            capture = root / "capture.wav"
            samples = media.audio_challenge(
                "ef" * 16,
                sample_rate=16000,
                duration_seconds=0.5,
            )
            media.write_wav(stimulus, samples, 16000)

            def fake_playrec(
                output: np.ndarray,
                **arguments: object,
            ) -> np.ndarray:
                self.assertEqual(arguments["device"], (2, 3))
                self.assertTrue(arguments["blocking"])
                return output.reshape(-1, 1)

            with (
                mock.patch.object(
                    media,
                    "resolve_audio_device",
                    side_effect=(2, 3),
                ),
                mock.patch.object(
                    media,
                    "audio_device_fingerprint",
                    side_effect=lambda index: {
                        "index": index,
                        "name": "input" if index == 2 else "output",
                    },
                ),
                mock.patch.object(
                    sounddevice,
                    "playrec",
                    side_effect=fake_playrec,
                ),
                mock.patch.object(
                    sounddevice,
                    "get_status",
                    return_value="",
                ),
            ):
                result = media.play_record_audio(
                    "input",
                    "output",
                    stimulus,
                    capture,
                    0.25,
                )
            self.assertTrue(result["clean_stream"])
            self.assertEqual(result["capture_samples"], 12000)
            self.assertTrue(capture.is_file())

    def test_playback_rejects_full_scale_or_unbounded_stimulus(self) -> None:
        with self.assertRaisesRegex(ValueError, "safe peak"):
            media.validate_playback_stimulus(
                np.array([32767], dtype=np.int16),
                48000,
            )
        with self.assertRaisesRegex(ValueError, "10 seconds"):
            media.validate_playback_stimulus(
                np.zeros(480001, dtype=np.int16),
                48000,
            )


if __name__ == "__main__":
    unittest.main()
