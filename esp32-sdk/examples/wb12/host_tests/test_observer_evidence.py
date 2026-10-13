import copy
import hashlib
import json
import os
import tempfile
import unittest
from datetime import UTC, datetime
from pathlib import Path
from unittest import mock

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
)

from bobe_hil.image import sign_manifest
from bobe_hil import observer_evidence
from bobe_hil.observer_evidence import (
    OBSERVER_CERTIFICATE_PURPOSE,
    OBSERVER_MOCK_CERTIFICATE_PURPOSE,
    OBSERVER_QUALIFICATION_PURPOSE,
    OBSERVER_RESULT_PURPOSE,
    PCM_MEDIA_TYPE,
    sha256_file,
    sign_observer_envelope,
    verify_observer_envelope,
    verify_observer_result,
)


class ObserverEvidenceTests(unittest.TestCase):
    now = datetime(2026, 7, 21, 12, 0, tzinfo=UTC)

    @staticmethod
    def write_keypair(
        root: Path,
        name: str,
    ) -> tuple[Path, Path]:
        key = Ed25519PrivateKey.generate()
        private = root / f"{name}-private.pem"
        public = root / f"{name}-public.pem"
        private.write_bytes(
            key.private_bytes(
                serialization.Encoding.PEM,
                serialization.PrivateFormat.PKCS8,
                serialization.NoEncryption(),
            )
        )
        public.write_bytes(
            key.public_key().public_bytes(
                serialization.Encoding.PEM,
                serialization.PublicFormat.SubjectPublicKeyInfo,
            )
        )
        return private, public

    @staticmethod
    def unsigned_sha256(value: dict[str, object]) -> str:
        unsigned = {
            key: item
            for key, item in value.items()
            if key not in {"signature", "signature_algorithm"}
        }
        return hashlib.sha256(
            json.dumps(
                unsigned,
                separators=(",", ":"),
                sort_keys=True,
            ).encode()
        ).hexdigest()

    def build_fixture(self, root: Path) -> dict[str, object]:
        manifest_private, manifest_public = self.write_keypair(
            root,
            "manifest",
        )
        observer_private, observer_public = self.write_keypair(
            root,
            "observer",
        )
        observer_id = "1" * 32
        deployment_sha256 = "2" * 64
        qualification_id = "3" * 32
        calibration_sha256 = "4" * 64
        certificate = sign_manifest(
            {
                "schema_version": 1,
                "purpose": OBSERVER_CERTIFICATE_PURPOSE,
                "observer_id": observer_id,
                "public_key_sha256": sha256_file(observer_public),
                "deployment_sha256": deployment_sha256,
                "created_at": "2026-01-01T00:00:00+00:00",
                "expires_at": "2027-01-01T00:00:00+00:00",
            },
            manifest_private,
        )
        qualification = sign_manifest(
            {
                "schema_version": 1,
                "purpose": OBSERVER_QUALIFICATION_PURPOSE,
                "observer_id": observer_id,
                "deployment_sha256": deployment_sha256,
                "qualification_id": qualification_id,
                "calibration_sha256": calibration_sha256,
                "wiring_sha256": "f" * 64,
                "created_at": "2026-01-01T00:00:00+00:00",
                "expires_at": "2026-12-31T23:59:59+00:00",
                "audio": {
                    "usb_vid": 0x1234,
                    "usb_pid": 0x5678,
                    "usb_serial": "AUDIO-FIXTURE-001",
                    "card_id": "Fixture",
                    "sample_rate": 48000,
                    "sample_format": "S16_LE",
                    "capture_channels": 2,
                    "witness_channel": 0,
                    "electrical_channel": 1,
                    "input_gain_millidb": 0,
                    "output_gain_millidb": -12000,
                },
                "thresholds": {
                    "max_clipped_samples": 0,
                    "max_crosstalk_correlation_ppm": 1000,
                    "max_downlink_latency_us": 100000,
                    "max_uplink_latency_us": 100000,
                    "max_xruns": 0,
                    "min_downlink_correlation_ppm": 900000,
                    "min_uplink_correlation_ppm": 900000,
                    "min_witness_snr_millidb": 20000,
                },
            },
            manifest_private,
        )
        artifact_root = root / "artifacts"
        artifact_root.mkdir()
        contents = {
            "bodylink-microphone.pcm": b"\x01\x00" * 16000,
            "bodylink-speaker.pcm": b"\x02\x00" * 72000,
            "bodylink-transcript.jsonl": b'{"type":"bodylink"}\n',
            "fixture-transcript.jsonl": b'{"type":"fixture"}\n',
            "observer-inputs.pcm": b"\x03\x00" * (384000 * 2),
            "uplink-stimulus.pcm": b"\x04\x00" * 48000,
        }
        sample_counts = {
            "bodylink-microphone.pcm": 16000,
            "bodylink-speaker.pcm": 72000,
            "bodylink-transcript.jsonl": 0,
            "fixture-transcript.jsonl": 0,
            "observer-inputs.pcm": 384000,
            "uplink-stimulus.pcm": 48000,
        }
        metadata = {
            "bodylink-microphone.pcm": (PCM_MEDIA_TYPE, 16000, 1),
            "bodylink-speaker.pcm": (PCM_MEDIA_TYPE, 24000, 1),
            "bodylink-transcript.jsonl": ("application/x-ndjson", 0, 0),
            "fixture-transcript.jsonl": ("application/x-ndjson", 0, 0),
            "observer-inputs.pcm": (PCM_MEDIA_TYPE, 48000, 2),
            "uplink-stimulus.pcm": (PCM_MEDIA_TYPE, 48000, 1),
        }
        artifacts = []
        for name, content in contents.items():
            path = artifact_root / name
            path.write_bytes(content)
            media_type, sample_rate, channels = metadata[name]
            artifacts.append(
                {
                    "name": name,
                    "media_type": media_type,
                    "sample_rate": sample_rate,
                    "channels": channels,
                    "sample_count": sample_counts[name],
                    "size": len(content),
                    "sha256": sha256_file(path),
                }
            )
        indexed = {artifact["name"]: artifact for artifact in artifacts}
        envelope = {
            "schema_version": 1,
            "purpose": OBSERVER_RESULT_PURPOSE,
            "status": "passed",
            "reason_code": "none",
            "started_unix_ms": 1784635200000,
            "finished_unix_ms": 1784635260000,
            "observer": {
                "observer_id": observer_id,
                "deployment_sha256": deployment_sha256,
                "boot_id": "9" * 32,
                "run_counter": 1,
            },
            "challenge": {
                "request_id": "5" * 32,
                "host_nonce": "6" * 64,
                "observer_nonce": "8" * 64,
                "contract_sha256": "7" * 64,
            },
            "device": {
                "device_id": "11111111-2222-4333-8444-555555555555",
                "boot_id": "a" * 32,
                "firmware_elf_sha256": "b" * 64,
                "client_spki_sha256": "c" * 64,
                "usb_serial_number": "d" * 12,
            },
            "fixture": {
                "board_id": "e" * 16,
                "firmware_version": "0.3.0",
                "requested_duration_ms": 1800,
                "transcript_sha256": indexed[
                    "fixture-transcript.jsonl"
                ]["sha256"],
                "actuation_started_monotonic_ns": 1_000_000_000,
                "actuation_finished_monotonic_ns": 2_800_000_000,
                "released": True,
            },
            "calibration": {
                "qualification_id": qualification_id,
                "qualification_sha256": self.unsigned_sha256(qualification),
                "calibration_sha256": calibration_sha256,
            },
            "bodylink": {
                "protocol_major": 1,
                "protocol_minor": 1,
                "control_transcript_sha256": indexed[
                    "bodylink-transcript.jsonl"
                ]["sha256"],
                "microphone_pcm_sha256": indexed[
                    "bodylink-microphone.pcm"
                ]["sha256"],
                "speaker_pcm_sha256": indexed[
                    "bodylink-speaker.pcm"
                ]["sha256"],
                "microphone_frames": 50,
                "speaker_frames": 150,
                "sequence_errors": 0,
            },
            "metrics": {
                "clipped_samples": 0,
                "crosstalk_correlation_ppm": 500,
                "downlink_correlation_ppm": 950000,
                "downlink_latency_us": 50000,
                "uplink_correlation_ppm": 960000,
                "uplink_latency_us": 40000,
                "witness_snr_millidb": 30000,
                "xruns": 0,
            },
            "cleanup": {
                "audio_closed": True,
                "bodylink_closed": True,
                "fixture_released": True,
                "outputs_silent": True,
            },
            "artifacts": artifacts,
        }
        signed_result = sign_observer_envelope(
            envelope,
            observer_private,
        )
        return {
            "manifest_private": manifest_private,
            "manifest_public": manifest_public,
            "observer_private": observer_private,
            "observer_public": observer_public,
            "certificate": certificate,
            "qualification": qualification,
            "artifact_root": artifact_root,
            "envelope": envelope,
            "signed_result": signed_result,
            "expected": {
                "expected_request_id": "5" * 32,
                "expected_host_nonce": "6" * 64,
                "expected_contract_sha256": "7" * 64,
                "expected_observer_id": observer_id,
                "expected_deployment_sha256": deployment_sha256,
                "expected_qualification_id": qualification_id,
                "expected_device_id": "11111111-2222-4333-8444-555555555555",
                "expected_device_boot_id": "a" * 32,
                "expected_firmware_elf_sha256": "b" * 64,
                "expected_client_spki_sha256": "c" * 64,
                "expected_usb_serial_number": "d" * 12,
                "expected_fixture_board_id": "e" * 16,
                "minimum_run_counter": 0,
                "now": self.now,
            },
        }

    def verify(self, fixture: dict[str, object], **overrides: object):
        expected = {**fixture["expected"], **overrides}
        with mock.patch.object(
            observer_evidence,
            "OBSERVER_MANIFEST_PUBLIC_KEY",
            fixture["manifest_public"],
        ):
            return verify_observer_result(
                fixture["signed_result"],
                fixture["certificate"],
                fixture["qualification"],
                fixture["observer_public"],
                fixture["artifact_root"],
                **expected,
            )

    def verify_envelope(
        self,
        fixture: dict[str, object],
        **overrides: object,
    ):
        expected = {**fixture["expected"], **overrides}
        with mock.patch.object(
            observer_evidence,
            "OBSERVER_MANIFEST_PUBLIC_KEY",
            fixture["manifest_public"],
        ):
            return verify_observer_envelope(
                fixture["signed_result"],
                fixture["certificate"],
                fixture["qualification"],
                fixture["observer_public"],
                **expected,
            )

    def test_valid_observer_pass_is_fresh_bound_and_auditable(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.build_fixture(Path(directory))
            envelope = self.verify_envelope(fixture)
            result = self.verify(fixture)
        self.assertTrue(envelope["candidate_pass"])
        self.assertFalse(envelope["artifact_bytes_verified"])
        self.assertTrue(result["verified"])
        self.assertTrue(result["passed"])
        self.assertEqual(result["artifact_count"], 6)
        self.assertEqual(result["run_counter"], 1)

    def test_rejects_replay_wrong_challenge_and_artifact_tampering(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.build_fixture(Path(directory))
            with self.assertRaisesRegex(ValueError, "values are invalid"):
                self.verify(fixture, minimum_run_counter=1)
            with self.assertRaisesRegex(ValueError, "values are invalid"):
                self.verify(
                    fixture,
                    expected_host_nonce="0" * 64,
                )
            path = fixture["artifact_root"] / "observer-inputs.pcm"
            path.write_bytes(b"\xff" * path.stat().st_size)
            with self.assertRaisesRegex(ValueError, "content does not match"):
                self.verify(fixture)

    def test_rejects_mock_root_purpose_and_envelope_tampering(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = self.build_fixture(root)
            mock_private, mock_public = self.write_keypair(root, "mock-root")
            mock_certificate = sign_manifest(
                {
                    key: value
                    for key, value in fixture["certificate"].items()
                    if key not in {"signature", "signature_algorithm"}
                },
                mock_private,
            )
            with (
                mock.patch.object(
                    observer_evidence,
                    "OBSERVER_MANIFEST_PUBLIC_KEY",
                    fixture["manifest_public"],
                ),
                self.assertRaisesRegex(ValueError, "signature is invalid"),
            ):
                verify_observer_result(
                    fixture["signed_result"],
                    mock_certificate,
                    fixture["qualification"],
                    fixture["observer_public"],
                    fixture["artifact_root"],
                    **fixture["expected"],
                )
            wrong_purpose = {
                key: value
                for key, value in fixture["certificate"].items()
                if key not in {"signature", "signature_algorithm"}
            }
            wrong_purpose["purpose"] = OBSERVER_MOCK_CERTIFICATE_PURPOSE
            wrong_purpose = sign_manifest(
                wrong_purpose,
                fixture["manifest_private"],
            )
            with (
                mock.patch.object(
                    observer_evidence,
                    "OBSERVER_MANIFEST_PUBLIC_KEY",
                    fixture["manifest_public"],
                ),
                self.assertRaisesRegex(ValueError, "values are invalid"),
            ):
                verify_observer_result(
                    fixture["signed_result"],
                    wrong_purpose,
                    fixture["qualification"],
                    fixture["observer_public"],
                    fixture["artifact_root"],
                    **fixture["expected"],
                )
            tampered = copy.deepcopy(fixture["signed_result"])
            tampered["envelope"]["metrics"]["xruns"] = 1
            with (
                mock.patch.object(
                    observer_evidence,
                    "OBSERVER_MANIFEST_PUBLIC_KEY",
                    fixture["manifest_public"],
                ),
                self.assertRaisesRegex(ValueError, "signature is invalid"),
            ):
                verify_observer_result(
                    tampered,
                    fixture["certificate"],
                    fixture["qualification"],
                    fixture["observer_public"],
                    fixture["artifact_root"],
                    **fixture["expected"],
                )
            self.assertTrue(mock_public.is_file())

    def test_failed_result_is_verified_but_never_authorized(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.build_fixture(Path(directory))
            envelope = copy.deepcopy(fixture["envelope"])
            envelope["status"] = "failed"
            envelope["reason_code"] = "bodylink-timeout"
            envelope["fixture"]["actuation_started_monotonic_ns"] = 0
            envelope["fixture"]["actuation_finished_monotonic_ns"] = 0
            envelope["artifacts"] = []
            envelope["bodylink"].update(
                {
                    "control_transcript_sha256": "0" * 64,
                    "microphone_pcm_sha256": "0" * 64,
                    "speaker_pcm_sha256": "0" * 64,
                    "microphone_frames": 0,
                    "speaker_frames": 0,
                }
            )
            fixture["signed_result"] = sign_observer_envelope(
                envelope,
                fixture["observer_private"],
            )
            result = self.verify(fixture)
        self.assertTrue(result["verified"])
        self.assertFalse(result["passed"])
        self.assertEqual(result["artifact_count"], 0)

    def test_pass_requires_cleanup_thresholds_and_integer_canonical_data(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.build_fixture(Path(directory))
            incomplete = copy.deepcopy(fixture["envelope"])
            incomplete["cleanup"]["outputs_silent"] = False
            fixture["signed_result"] = sign_observer_envelope(
                incomplete,
                fixture["observer_private"],
            )
            with self.assertRaisesRegex(
                ValueError,
                "does not satisfy qualification",
            ):
                self.verify(fixture)

            floating = copy.deepcopy(fixture["envelope"])
            floating["metrics"]["uplink_correlation_ppm"] = 0.95
            with self.assertRaisesRegex(
                ValueError,
                "unsupported canonical value",
            ):
                sign_observer_envelope(
                    floating,
                    fixture["observer_private"],
                )

            short_audio = copy.deepcopy(fixture["envelope"])
            short_audio["bodylink"]["microphone_frames"] = 1
            fixture["signed_result"] = sign_observer_envelope(
                short_audio,
                fixture["observer_private"],
            )
            with self.assertRaisesRegex(
                ValueError,
                "does not satisfy qualification",
            ):
                self.verify(fixture)

    def test_rejects_action_timing_and_pcm_format_drift(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.build_fixture(Path(directory))
            zero_duration = copy.deepcopy(fixture["envelope"])
            zero_duration["fixture"]["actuation_finished_monotonic_ns"] = (
                zero_duration["fixture"]["actuation_started_monotonic_ns"]
            )
            fixture["signed_result"] = sign_observer_envelope(
                zero_duration,
                fixture["observer_private"],
            )
            with self.assertRaisesRegex(
                ValueError,
                "does not satisfy qualification",
            ):
                self.verify(fixture)

            bad_format = {
                key: value
                for key, value in fixture["qualification"].items()
                if key not in {"signature", "signature_algorithm"}
            }
            bad_format["audio"] = {
                **bad_format["audio"],
                "sample_format": "S16_BE",
            }
            fixture["qualification"] = sign_manifest(
                bad_format,
                fixture["manifest_private"],
            )
            fixture["signed_result"] = sign_observer_envelope(
                fixture["envelope"],
                fixture["observer_private"],
            )
            with self.assertRaisesRegex(ValueError, "values are invalid"):
                self.verify(fixture)

    def test_rejects_non_regular_artifact_without_blocking(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.build_fixture(Path(directory))
            path = fixture["artifact_root"] / "observer-inputs.pcm"
            path.unlink()
            os.mkfifo(path)
            with self.assertRaisesRegex(
                ValueError,
                "size or type is invalid",
            ):
                self.verify(fixture)

    def test_rejects_fifo_public_key_without_blocking(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.build_fixture(Path(directory))
            fixture["observer_public"].unlink()
            os.mkfifo(fixture["observer_public"])
            with self.assertRaisesRegex(
                ValueError,
                "missing or unsafe",
            ):
                self.verify(fixture)

    def test_rejects_deep_unsigned_envelope_without_recursion_failure(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.build_fixture(Path(directory))
            deep: list[object] = []
            cursor = deep
            for _ in range(32):
                child: list[object] = []
                cursor.append(child)
                cursor = child
            malicious = copy.deepcopy(fixture["signed_result"])
            malicious["envelope"]["metrics"]["clipped_samples"] = deep
            fixture["signed_result"] = malicious
            with self.assertRaisesRegex(ValueError, "depth bound"):
                self.verify(fixture)

    def test_public_key_is_read_once_and_reused_after_path_swap(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = self.build_fixture(root)
            _, attacker_public = self.write_keypair(root, "attacker")
            original_reader = observer_evidence.read_observer_public_key

            def read_then_swap(path: Path):
                loaded = original_reader(path)
                path.write_bytes(attacker_public.read_bytes())
                return loaded

            with mock.patch.object(
                observer_evidence,
                "read_observer_public_key",
                side_effect=read_then_swap,
            ) as reader:
                result = self.verify(fixture)
            self.assertTrue(result["passed"])
            reader.assert_called_once_with(fixture["observer_public"])

    def test_rejects_unbounded_unsigned_certificate_and_qualification(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as directory:
            fixture = self.build_fixture(Path(directory))
            oversized_certificate = {
                **fixture["certificate"],
                "signature": "A" * 100_000,
            }
            fixture["certificate"] = oversized_certificate
            with self.assertRaisesRegex(ValueError, "bounded printable ASCII"):
                self.verify(fixture)

            second = Path(directory) / "second"
            second.mkdir()
            fixture = self.build_fixture(second)
            deep: list[object] = []
            cursor = deep
            for _ in range(32):
                child: list[object] = []
                cursor.append(child)
                cursor = child
            qualification = copy.deepcopy(fixture["qualification"])
            qualification["thresholds"] = deep
            fixture["qualification"] = qualification
            with self.assertRaisesRegex(ValueError, "depth bound"):
                self.verify(fixture)


if __name__ == "__main__":
    unittest.main()
