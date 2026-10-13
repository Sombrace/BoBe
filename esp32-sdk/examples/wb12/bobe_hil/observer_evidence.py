import base64
import binascii
import hashlib
import json
import os
import re
import stat
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
    Ed25519PublicKey,
)

from bobe_hil.image import SIGNING_PUBLIC_KEY, verify_manifest_signature
from bobe_hil.paths import sha256_file
from bobe_hil.observer_protocol import (
    ARTIFACT_MAX_BYTES,
    MAX_ARTIFACT_BYTES,
    MAX_TOTAL_ARTIFACT_BYTES,
)


OBSERVER_SCHEMA = 1
OBSERVER_CERTIFICATE_PURPOSE = "bobe-independent-observer-evidence"
OBSERVER_MOCK_CERTIFICATE_PURPOSE = "bobe-mock-observer-evidence"
OBSERVER_QUALIFICATION_PURPOSE = "bobe-independent-observer-qualification"
OBSERVER_RESULT_PURPOSE = "bobe-physical-bodylink-audio-loop"
OBSERVER_SIGNATURE_ALGORITHM = "ed25519"
OBSERVER_SIGNATURE_DOMAIN = b"BOBE-OBSERVER-EVIDENCE-V1\x00"
OBSERVER_MANIFEST_PUBLIC_KEY = SIGNING_PUBLIC_KEY
PCM_MEDIA_TYPE = "application/vnd.bobe.pcm-s16le"
PCM_SAMPLE_FORMAT = "S16_LE"
EXPECTED_ACTION_DURATION_MS = 1800
MIN_ACTION_DURATION_NS = 1_700_000_000
MAX_ACTION_DURATION_NS = 2_200_000_000
MAX_CANONICAL_DEPTH = 16
MAX_CANONICAL_NODES = 4096
MAX_CANONICAL_BYTES = 1024 * 1024
MAX_SIGNED_MANIFEST_BYTES = 64 * 1024
UPLINK_STIMULUS_SAMPLES = 48_000
MIN_MICROPHONE_FRAMES = 50
MAX_MICROPHONE_FRAMES = 500
SPEAKER_FRAMES = 150
OBSERVER_CAPTURE_SAMPLES = 8 * 48_000
ARTIFACT_CONTRACT = {
    "bodylink-microphone.pcm": (PCM_MEDIA_TYPE, 16000, 1),
    "bodylink-speaker.pcm": (PCM_MEDIA_TYPE, 24000, 1),
    "bodylink-transcript.jsonl": ("application/x-ndjson", 0, 0),
    "fixture-transcript.jsonl": ("application/x-ndjson", 0, 0),
    "observer-inputs.pcm": (PCM_MEDIA_TYPE, 48000, 2),
    "uplink-stimulus.pcm": (PCM_MEDIA_TYPE, 48000, 1),
}
THRESHOLD_FIELDS = {
    "max_clipped_samples",
    "max_crosstalk_correlation_ppm",
    "max_downlink_latency_us",
    "max_uplink_latency_us",
    "max_xruns",
    "min_downlink_correlation_ppm",
    "min_uplink_correlation_ppm",
    "min_witness_snr_millidb",
}
METRIC_FIELDS = {
    "clipped_samples",
    "crosstalk_correlation_ppm",
    "downlink_correlation_ppm",
    "downlink_latency_us",
    "uplink_correlation_ppm",
    "uplink_latency_us",
    "witness_snr_millidb",
    "xruns",
}


def require_fields(
    value: Any,
    expected: set[str],
    description: str,
) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != expected:
        raise ValueError(f"{description} fields are invalid")
    return value


def valid_hex(value: Any, length: int) -> bool:
    return (
        isinstance(value, str)
        and len(value) == length
        and re.fullmatch(r"[0-9a-f]+", value) is not None
    )


def valid_ascii(value: Any, minimum: int, maximum: int) -> bool:
    if not isinstance(value, str) or not minimum <= len(value) <= maximum:
        return False
    try:
        value.encode("ascii")
    except UnicodeEncodeError:
        return False
    return all(character.isprintable() for character in value)


def parse_time(value: Any, description: str) -> datetime:
    if not isinstance(value, str):
        raise ValueError(f"{description} timestamp is invalid")
    try:
        parsed = datetime.fromisoformat(value)
    except ValueError as error:
        raise ValueError(f"{description} timestamp is invalid") from error
    if parsed.tzinfo is None:
        raise ValueError(f"{description} timestamp lacks a timezone")
    return parsed.astimezone(UTC)


def checked_file(
    path: Path,
    description: str,
    *,
    maximum: int | None = None,
) -> Path:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{description} is missing or unsafe")
    if maximum is not None and path.stat().st_size > maximum:
        raise ValueError(f"{description} exceeds its size bound")
    return path


def hash_bounded_regular_file(
    path: Path,
    *,
    expected_size: int,
) -> str:
    try:
        descriptor = os.open(
            path,
            os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
        )
    except OSError as error:
        raise ValueError("observer artifact is missing or unsafe") from error
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_size != expected_size
            or expected_size <= 0
            or expected_size > MAX_ARTIFACT_BYTES
        ):
            raise ValueError("observer artifact size or type is invalid")
        digest = hashlib.sha256()
        remaining = expected_size
        while remaining:
            chunk = os.read(descriptor, min(1024 * 1024, remaining))
            if not chunk:
                raise ValueError("observer artifact ended before declared size")
            digest.update(chunk)
            remaining -= len(chunk)
        final_metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(final_metadata.st_mode)
            or final_metadata.st_size != expected_size
        ):
            raise ValueError("observer artifact changed while reading")
        return digest.hexdigest()
    finally:
        os.close(descriptor)


def validate_canonical_subset(
    value: Any,
    path: str = "envelope",
    *,
    depth: int = 0,
    state: list[int] | None = None,
) -> None:
    if depth > MAX_CANONICAL_DEPTH:
        raise ValueError("observer canonical value exceeds its depth bound")
    if state is None:
        state = [0]
    state[0] += 1
    if state[0] > MAX_CANONICAL_NODES:
        raise ValueError("observer canonical value exceeds its node bound")
    if value is None or type(value) is bool:
        return
    if type(value) is int:
        if not -(2**63) <= value < 2**63:
            raise ValueError(f"{path} integer is outside the signed 64-bit range")
        return
    if isinstance(value, str):
        if not valid_ascii(value, 0, 4096):
            raise ValueError(f"{path} string is not bounded printable ASCII")
        return
    if isinstance(value, list):
        if len(value) > 64:
            raise ValueError(f"{path} list exceeds its item bound")
        for index, item in enumerate(value):
            validate_canonical_subset(
                item,
                f"{path}[{index}]",
                depth=depth + 1,
                state=state,
            )
        return
    if isinstance(value, dict):
        if len(value) > 64:
            raise ValueError(f"{path} object exceeds its field bound")
        for key, item in value.items():
            if not isinstance(key, str) or re.fullmatch(
                r"[a-z][a-z0-9_]{0,63}",
                key,
            ) is None:
                raise ValueError(f"{path} key is invalid")
            validate_canonical_subset(
                item,
                f"{path}.{key}",
                depth=depth + 1,
                state=state,
            )
        return
    raise ValueError(f"{path} contains an unsupported canonical value")


def canonical_observer_envelope(envelope: dict[str, Any]) -> bytes:
    validate_canonical_subset(envelope)
    encoded = json.dumps(
        envelope,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("ascii")
    if len(encoded) > MAX_CANONICAL_BYTES:
        raise ValueError("observer canonical envelope exceeds its byte bound")
    return OBSERVER_SIGNATURE_DOMAIN + encoded


def validate_signed_object(
    value: dict[str, Any],
    description: str,
) -> None:
    validate_canonical_subset(value, description)
    encoded = json.dumps(
        value,
        ensure_ascii=True,
        separators=(",", ":"),
        sort_keys=True,
    ).encode("ascii")
    if len(encoded) > MAX_SIGNED_MANIFEST_BYTES:
        raise ValueError(f"{description} exceeds its byte bound")


def validate_ed25519_signature_fields(
    value: dict[str, Any],
    description: str,
) -> None:
    if value.get("signature_algorithm") != OBSERVER_SIGNATURE_ALGORITHM:
        raise ValueError(f"{description} signature algorithm is invalid")
    encoded = value.get("signature")
    if not isinstance(encoded, str) or len(encoded) != 88:
        raise ValueError(f"{description} signature is malformed")
    try:
        decoded = base64.b64decode(encoded, validate=True)
    except (ValueError, binascii.Error) as error:
        raise ValueError(f"{description} signature is malformed") from error
    if len(decoded) != 64:
        raise ValueError(f"{description} signature is malformed")


def sign_observer_envelope(
    envelope: dict[str, Any],
    private_key_path: Path,
) -> dict[str, Any]:
    checked_file(private_key_path, "observer signing key", maximum=16 * 1024)
    try:
        loaded = serialization.load_pem_private_key(
            private_key_path.read_bytes(),
            password=None,
        )
    except (TypeError, ValueError) as error:
        raise ValueError("observer signing key is invalid") from error
    if not isinstance(loaded, Ed25519PrivateKey):
        raise ValueError("observer signing key is not Ed25519")
    signature = loaded.sign(canonical_observer_envelope(envelope))
    return {
        "envelope": envelope,
        "signature_algorithm": OBSERVER_SIGNATURE_ALGORITHM,
        "signature": base64.b64encode(signature).decode("ascii"),
    }


def read_observer_public_key(
    path: Path,
) -> tuple[bytes, Ed25519PublicKey]:
    try:
        descriptor = os.open(
            path,
            os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
        )
    except OSError as error:
        raise ValueError("observer public key is missing or unsafe") from error
    try:
        metadata = os.fstat(descriptor)
        if (
            not stat.S_ISREG(metadata.st_mode)
            or metadata.st_size <= 0
            or metadata.st_size > 16 * 1024
        ):
            raise ValueError("observer public key is missing or unsafe")
        chunks = bytearray()
        while len(chunks) <= 16 * 1024:
            chunk = os.read(descriptor, 4096)
            if not chunk:
                break
            chunks.extend(chunk)
        if len(chunks) != metadata.st_size:
            raise ValueError("observer public key changed while reading")
    finally:
        os.close(descriptor)
    key_bytes = bytes(chunks)
    try:
        loaded = serialization.load_pem_public_key(key_bytes)
    except ValueError as error:
        raise ValueError("observer public key is invalid") from error
    if not isinstance(loaded, Ed25519PublicKey):
        raise ValueError("observer public key is not Ed25519")
    return key_bytes, loaded


def verify_certificate(
    certificate: dict[str, Any],
    observer_public_key_bytes: bytes,
    *,
    now: datetime,
) -> dict[str, Any]:
    require_fields(
        certificate,
        {
            "created_at",
            "deployment_sha256",
            "expires_at",
            "observer_id",
            "public_key_sha256",
            "purpose",
            "schema_version",
            "signature",
            "signature_algorithm",
        },
        "observer certificate",
    )
    validate_signed_object(certificate, "observer certificate")
    validate_ed25519_signature_fields(certificate, "observer certificate")
    verify_manifest_signature(certificate, OBSERVER_MANIFEST_PUBLIC_KEY)
    created = parse_time(certificate["created_at"], "observer certificate")
    expires = parse_time(certificate["expires_at"], "observer certificate")
    if (
        type(certificate["schema_version"]) is not int
        or certificate["schema_version"] != OBSERVER_SCHEMA
        or certificate["purpose"] != OBSERVER_CERTIFICATE_PURPOSE
        or not valid_hex(certificate["observer_id"], 32)
        or not valid_hex(certificate["deployment_sha256"], 64)
        or not valid_hex(certificate["public_key_sha256"], 64)
        or not created <= now <= expires
        or expires <= created
    ):
        raise ValueError("observer certificate values are invalid")
    if (
        hashlib.sha256(observer_public_key_bytes).hexdigest()
        != certificate["public_key_sha256"]
    ):
        raise ValueError("observer public key does not match its certificate")
    return certificate


def verify_qualification(
    qualification: dict[str, Any],
    *,
    observer_id: str,
    deployment_sha256: str,
    now: datetime,
) -> dict[str, Any]:
    require_fields(
        qualification,
        {
            "audio",
            "calibration_sha256",
            "created_at",
            "deployment_sha256",
            "expires_at",
            "observer_id",
            "purpose",
            "qualification_id",
            "schema_version",
            "signature",
            "signature_algorithm",
            "thresholds",
            "wiring_sha256",
        },
        "observer qualification",
    )
    validate_signed_object(qualification, "observer qualification")
    validate_ed25519_signature_fields(
        qualification,
        "observer qualification",
    )
    verify_manifest_signature(qualification, OBSERVER_MANIFEST_PUBLIC_KEY)
    audio = require_fields(
        qualification["audio"],
        {
            "capture_channels",
            "card_id",
            "electrical_channel",
            "input_gain_millidb",
            "output_gain_millidb",
            "sample_format",
            "sample_rate",
            "usb_pid",
            "usb_serial",
            "usb_vid",
            "witness_channel",
        },
        "observer qualification audio",
    )
    thresholds = require_fields(
        qualification["thresholds"],
        THRESHOLD_FIELDS,
        "observer qualification thresholds",
    )
    created = parse_time(qualification["created_at"], "observer qualification")
    expires = parse_time(qualification["expires_at"], "observer qualification")
    if (
        type(qualification["schema_version"]) is not int
        or qualification["schema_version"] != OBSERVER_SCHEMA
        or qualification["purpose"] != OBSERVER_QUALIFICATION_PURPOSE
        or qualification["observer_id"] != observer_id
        or qualification["deployment_sha256"] != deployment_sha256
        or not valid_hex(qualification["qualification_id"], 32)
        or not valid_hex(qualification["calibration_sha256"], 64)
        or not valid_hex(qualification["wiring_sha256"], 64)
        or not created <= now <= expires
        or expires <= created
        or type(audio["usb_vid"]) is not int
        or not 1 <= audio["usb_vid"] <= 0xFFFF
        or type(audio["usb_pid"]) is not int
        or not 1 <= audio["usb_pid"] <= 0xFFFF
        or not valid_ascii(audio["usb_serial"], 1, 128)
        or not valid_ascii(audio["card_id"], 1, 64)
        or audio["sample_rate"] != 48000
        or audio["sample_format"] != PCM_SAMPLE_FORMAT
        or audio["capture_channels"] != 2
        or audio["witness_channel"] != 0
        or audio["electrical_channel"] != 1
        or type(audio["input_gain_millidb"]) is not int
        or not -120000 <= audio["input_gain_millidb"] <= 120000
        or type(audio["output_gain_millidb"]) is not int
        or not -120000 <= audio["output_gain_millidb"] <= 120000
    ):
        raise ValueError("observer qualification values are invalid")
    if any(type(value) is not int or value < 0 for value in thresholds.values()):
        raise ValueError("observer qualification thresholds are invalid")
    for field in (
        "max_crosstalk_correlation_ppm",
        "min_downlink_correlation_ppm",
        "min_uplink_correlation_ppm",
    ):
        if thresholds[field] > 1_000_000:
            raise ValueError("observer correlation threshold is invalid")
    if (
        thresholds["max_clipped_samples"] > 10_000_000
        or thresholds["max_downlink_latency_us"] > 10_000_000
        or thresholds["max_uplink_latency_us"] > 10_000_000
        or thresholds["max_xruns"] > 1000
        or thresholds["min_witness_snr_millidb"] > 200_000
    ):
        raise ValueError("observer qualification threshold is outside bounds")
    return qualification


def verify_artifacts(
    artifact_root: Path | None,
    artifacts: Any,
    *,
    observer_sample_rate: int,
    require_complete: bool,
) -> dict[str, dict[str, Any]]:
    if (
        artifact_root is not None
        and (artifact_root.is_symlink() or not artifact_root.is_dir())
    ):
        raise ValueError("observer artifact root is missing or unsafe")
    if (
        not isinstance(artifacts, list)
        or len(artifacts) > len(ARTIFACT_CONTRACT)
        or (require_complete and len(artifacts) != len(ARTIFACT_CONTRACT))
    ):
        raise ValueError("observer artifact manifest is invalid")
    indexed: dict[str, dict[str, Any]] = {}
    total_size = 0
    for artifact in artifacts:
        artifact = require_fields(
            artifact,
            {
                "channels",
                "media_type",
                "name",
                "sample_count",
                "sample_rate",
                "sha256",
                "size",
            },
            "observer artifact",
        )
        name = artifact["name"]
        if name not in ARTIFACT_CONTRACT or name in indexed:
            raise ValueError("observer artifact name is invalid or duplicated")
        media_type, sample_rate, channels = ARTIFACT_CONTRACT[name]
        if name == "observer-inputs.pcm":
            sample_rate = observer_sample_rate
        if (
            artifact["media_type"] != media_type
            or artifact["sample_rate"] != sample_rate
            or artifact["channels"] != channels
            or type(artifact["size"]) is not int
            or not 0
            < artifact["size"]
            <= ARTIFACT_MAX_BYTES[name]
            or type(artifact["sample_count"]) is not int
            or artifact["sample_count"] < 0
            or not valid_hex(artifact["sha256"], 64)
        ):
            raise ValueError("observer artifact metadata is invalid")
        if media_type == PCM_MEDIA_TYPE and artifact["sample_count"] <= 0:
            raise ValueError("observer PCM artifact has no samples")
        if (
            media_type == PCM_MEDIA_TYPE
            and artifact["size"]
            != artifact["sample_count"] * artifact["channels"] * 2
        ):
            raise ValueError("observer PCM artifact size is invalid")
        if media_type != PCM_MEDIA_TYPE and artifact["sample_count"] != 0:
            raise ValueError("observer transcript sample count is invalid")
        if artifact_root is not None:
            path = artifact_root / name
            if (
                path.parent != artifact_root
                or hash_bounded_regular_file(
                    path,
                    expected_size=artifact["size"],
                )
                != artifact["sha256"]
            ):
                raise ValueError(
                    "observer artifact content does not match manifest"
                )
        total_size += artifact["size"]
        indexed[name] = artifact
    if require_complete and set(indexed) != set(ARTIFACT_CONTRACT):
        raise ValueError("observer artifact set is invalid")
    if total_size > MAX_TOTAL_ARTIFACT_BYTES:
        raise ValueError("observer artifacts exceed their aggregate bound")
    return indexed


def _verify_observer_result(
    signed_result: dict[str, Any],
    certificate: dict[str, Any],
    qualification: dict[str, Any],
    observer_public_key: Path,
    artifact_root: Path | None,
    *,
    expected_request_id: str,
    expected_host_nonce: str,
    expected_contract_sha256: str,
    expected_observer_id: str,
    expected_deployment_sha256: str,
    expected_qualification_id: str,
    expected_device_id: str,
    expected_device_boot_id: str,
    expected_firmware_elf_sha256: str,
    expected_client_spki_sha256: str,
    expected_usb_serial_number: str,
    expected_fixture_board_id: str,
    minimum_run_counter: int,
    now: datetime | None = None,
) -> dict[str, Any]:
    now = (now or datetime.now(UTC)).astimezone(UTC)
    if (
        not valid_hex(expected_request_id, 32)
        or not valid_hex(expected_host_nonce, 64)
        or not valid_hex(expected_contract_sha256, 64)
        or not valid_hex(expected_observer_id, 32)
        or not valid_hex(expected_deployment_sha256, 64)
        or not valid_hex(expected_qualification_id, 32)
        or not valid_ascii(expected_device_id, 1, 64)
        or not valid_hex(expected_device_boot_id, 32)
        or not valid_hex(expected_firmware_elf_sha256, 64)
        or not valid_hex(expected_client_spki_sha256, 64)
        or not valid_hex(expected_usb_serial_number, 12)
        or not valid_hex(expected_fixture_board_id, 16)
        or type(minimum_run_counter) is not int
        or minimum_run_counter < 0
    ):
        raise ValueError("observer expected binding is invalid")
    observer_public_key_bytes, parsed_observer_public_key = (
        read_observer_public_key(observer_public_key)
    )
    certificate = verify_certificate(
        certificate,
        observer_public_key_bytes,
        now=now,
    )
    qualification = verify_qualification(
        qualification,
        observer_id=str(certificate["observer_id"]),
        deployment_sha256=str(certificate["deployment_sha256"]),
        now=now,
    )
    signed_result = require_fields(
        signed_result,
        {"envelope", "signature", "signature_algorithm"},
        "observer signed result",
    )
    if signed_result["signature_algorithm"] != OBSERVER_SIGNATURE_ALGORITHM:
        raise ValueError("observer result signature algorithm is invalid")
    encoded_signature = signed_result["signature"]
    if not isinstance(encoded_signature, str) or len(encoded_signature) != 88:
        raise ValueError("observer result signature is malformed")
    try:
        signature = base64.b64decode(encoded_signature, validate=True)
    except (ValueError, binascii.Error) as error:
        raise ValueError("observer result signature is malformed") from error
    if len(signature) != 64:
        raise ValueError("observer result signature is malformed")
    envelope = require_fields(
        signed_result["envelope"],
        {
            "artifacts",
            "bodylink",
            "calibration",
            "challenge",
            "cleanup",
            "device",
            "finished_unix_ms",
            "fixture",
            "metrics",
            "observer",
            "purpose",
            "reason_code",
            "schema_version",
            "started_unix_ms",
            "status",
        },
        "observer evidence envelope",
    )
    try:
        parsed_observer_public_key.verify(
            signature,
            canonical_observer_envelope(envelope),
        )
    except InvalidSignature as error:
        raise ValueError("observer result signature is invalid") from error
    observer = require_fields(
        envelope["observer"],
        {
            "boot_id",
            "deployment_sha256",
            "observer_id",
            "run_counter",
        },
        "observer identity",
    )
    challenge = require_fields(
        envelope["challenge"],
        {
            "contract_sha256",
            "host_nonce",
            "observer_nonce",
            "request_id",
        },
        "observer challenge",
    )
    device = require_fields(
        envelope["device"],
        {
            "boot_id",
            "client_spki_sha256",
            "device_id",
            "firmware_elf_sha256",
            "usb_serial_number",
        },
        "observer device identity",
    )
    fixture = require_fields(
        envelope["fixture"],
        {
            "actuation_finished_monotonic_ns",
            "actuation_started_monotonic_ns",
            "board_id",
            "firmware_version",
            "requested_duration_ms",
            "released",
            "transcript_sha256",
        },
        "observer fixture evidence",
    )
    calibration = require_fields(
        envelope["calibration"],
        {"calibration_sha256", "qualification_id", "qualification_sha256"},
        "observer calibration binding",
    )
    bodylink = require_fields(
        envelope["bodylink"],
        {
            "control_transcript_sha256",
            "microphone_frames",
            "microphone_pcm_sha256",
            "protocol_major",
            "protocol_minor",
            "sequence_errors",
            "speaker_frames",
            "speaker_pcm_sha256",
        },
        "observer BodyLink evidence",
    )
    metrics = require_fields(
        envelope["metrics"],
        METRIC_FIELDS,
        "observer metrics",
    )
    cleanup = require_fields(
        envelope["cleanup"],
        {
            "audio_closed",
            "bodylink_closed",
            "fixture_released",
            "outputs_silent",
        },
        "observer cleanup",
    )
    if (
        type(envelope["schema_version"]) is not int
        or envelope["schema_version"] != OBSERVER_SCHEMA
        or envelope["purpose"] != OBSERVER_RESULT_PURPOSE
        or envelope["status"] not in {"passed", "failed"}
        or not valid_ascii(envelope["reason_code"], 1, 64)
        or type(envelope["started_unix_ms"]) is not int
        or type(envelope["finished_unix_ms"]) is not int
        or envelope["started_unix_ms"] < 0
        or not 0
        <= envelope["finished_unix_ms"] - envelope["started_unix_ms"]
        <= 300_000
        or observer["observer_id"] != expected_observer_id
        or observer["observer_id"] != certificate["observer_id"]
        or observer["deployment_sha256"] != expected_deployment_sha256
        or observer["deployment_sha256"] != certificate["deployment_sha256"]
        or not valid_hex(observer["boot_id"], 32)
        or type(observer["run_counter"]) is not int
        or observer["run_counter"] <= minimum_run_counter
        or challenge["request_id"] != expected_request_id
        or challenge["host_nonce"] != expected_host_nonce
        or challenge["contract_sha256"] != expected_contract_sha256
        or not valid_hex(challenge["request_id"], 32)
        or not valid_hex(challenge["host_nonce"], 64)
        or not valid_hex(challenge["observer_nonce"], 64)
        or not valid_hex(challenge["contract_sha256"], 64)
        or device["device_id"] != expected_device_id
        or device["boot_id"] != expected_device_boot_id
        or device["client_spki_sha256"] != expected_client_spki_sha256
        or device["firmware_elf_sha256"] != expected_firmware_elf_sha256
        or device["usb_serial_number"] != expected_usb_serial_number
        or fixture["board_id"] != expected_fixture_board_id
        or fixture["firmware_version"] != "0.3.0"
        or fixture["requested_duration_ms"] != EXPECTED_ACTION_DURATION_MS
        or type(fixture["released"]) is not bool
        or not valid_hex(fixture["transcript_sha256"], 64)
        or type(fixture["actuation_started_monotonic_ns"]) is not int
        or type(fixture["actuation_finished_monotonic_ns"]) is not int
        or fixture["actuation_started_monotonic_ns"] < 0
        or fixture["actuation_finished_monotonic_ns"]
        < fixture["actuation_started_monotonic_ns"]
        or fixture["actuation_finished_monotonic_ns"]
        - fixture["actuation_started_monotonic_ns"]
        > 300_000_000_000
        or calibration["qualification_id"] != expected_qualification_id
        or calibration["qualification_id"] != qualification["qualification_id"]
        or calibration["calibration_sha256"]
        != qualification["calibration_sha256"]
        or not valid_hex(calibration["qualification_sha256"], 64)
        or bodylink["protocol_major"] != 1
        or bodylink["protocol_minor"] != 1
        or type(bodylink["microphone_frames"]) is not int
        or bodylink["microphone_frames"] < 0
        or type(bodylink["speaker_frames"]) is not int
        or bodylink["speaker_frames"] < 0
        or type(bodylink["sequence_errors"]) is not int
        or bodylink["sequence_errors"] < 0
        or not valid_hex(bodylink["control_transcript_sha256"], 64)
        or not valid_hex(bodylink["microphone_pcm_sha256"], 64)
        or not valid_hex(bodylink["speaker_pcm_sha256"], 64)
        or any(type(value) is not int or value < 0 for value in metrics.values())
        or metrics["crosstalk_correlation_ppm"] > 1_000_000
        or metrics["downlink_correlation_ppm"] > 1_000_000
        or metrics["uplink_correlation_ppm"] > 1_000_000
        or metrics["downlink_latency_us"] > 10_000_000
        or metrics["uplink_latency_us"] > 10_000_000
        or metrics["witness_snr_millidb"] > 200_000
        or any(type(value) is not bool for value in cleanup.values())
    ):
        raise ValueError("observer evidence values are invalid")

    unsigned_qualification = {
        key: value
        for key, value in qualification.items()
        if key not in {"signature", "signature_algorithm"}
    }
    qualification_sha256 = hashlib.sha256(
        json.dumps(
            unsigned_qualification,
            separators=(",", ":"),
            sort_keys=True,
        ).encode()
    ).hexdigest()
    if calibration["qualification_sha256"] != qualification_sha256:
        raise ValueError("observer qualification digest does not match")

    artifacts = verify_artifacts(
        artifact_root,
        envelope["artifacts"],
        observer_sample_rate=int(qualification["audio"]["sample_rate"]),
        require_complete=envelope["status"] == "passed",
    )
    artifact_bindings = {
        "bodylink-microphone.pcm": bodylink["microphone_pcm_sha256"],
        "bodylink-speaker.pcm": bodylink["speaker_pcm_sha256"],
        "bodylink-transcript.jsonl": bodylink[
            "control_transcript_sha256"
        ],
        "fixture-transcript.jsonl": fixture["transcript_sha256"],
    }
    if any(
        name in artifacts and digest != artifacts[name]["sha256"]
        for name, digest in artifact_bindings.items()
    ):
        raise ValueError("observer transcript or media binding is invalid")

    passed_shape = (
        fixture["released"] is True
        and MIN_ACTION_DURATION_NS
        <= fixture["actuation_finished_monotonic_ns"]
        - fixture["actuation_started_monotonic_ns"]
        <= MAX_ACTION_DURATION_NS
        and MIN_MICROPHONE_FRAMES
        <= bodylink["microphone_frames"]
        <= MAX_MICROPHONE_FRAMES
        and bodylink["speaker_frames"] == SPEAKER_FRAMES
        and bodylink["sequence_errors"] == 0
        and all(cleanup.values())
        and artifacts["bodylink-microphone.pcm"]["sample_count"]
        == bodylink["microphone_frames"] * 320
        and artifacts["bodylink-speaker.pcm"]["sample_count"]
        == bodylink["speaker_frames"] * 480
        and artifacts["uplink-stimulus.pcm"]["sample_count"]
        == UPLINK_STIMULUS_SAMPLES
        and artifacts["observer-inputs.pcm"]["sample_count"]
        == OBSERVER_CAPTURE_SAMPLES
    ) if envelope["status"] == "passed" else False

    thresholds = qualification["thresholds"]
    thresholds_pass = (
        metrics["clipped_samples"] <= thresholds["max_clipped_samples"]
        and metrics["crosstalk_correlation_ppm"]
        <= thresholds["max_crosstalk_correlation_ppm"]
        and metrics["downlink_correlation_ppm"]
        >= thresholds["min_downlink_correlation_ppm"]
        and metrics["downlink_latency_us"]
        <= thresholds["max_downlink_latency_us"]
        and metrics["uplink_correlation_ppm"]
        >= thresholds["min_uplink_correlation_ppm"]
        and metrics["uplink_latency_us"]
        <= thresholds["max_uplink_latency_us"]
        and metrics["witness_snr_millidb"]
        >= thresholds["min_witness_snr_millidb"]
        and metrics["xruns"] <= thresholds["max_xruns"]
    )
    passed = (
        envelope["status"] == "passed"
        and envelope["reason_code"] == "none"
        and passed_shape
        and thresholds_pass
    )
    if envelope["status"] == "passed" and not passed:
        raise ValueError("observer pass does not satisfy qualification thresholds")
    if envelope["status"] == "failed" and envelope["reason_code"] == "none":
        raise ValueError("observer failure lacks a reason code")
    return {
        "verified": True,
        "candidate_pass": passed,
        "artifact_bytes_verified": artifact_root is not None,
        "purpose": envelope["purpose"],
        "observer_id": observer["observer_id"],
        "deployment_sha256": observer["deployment_sha256"],
        "run_counter": observer["run_counter"],
        "observer_nonce": challenge["observer_nonce"],
        "qualification_id": qualification["qualification_id"],
        "qualification_sha256": qualification_sha256,
        "artifact_count": len(artifacts),
        "artifact_total_bytes": sum(
            int(artifact["size"]) for artifact in artifacts.values()
        ),
        "envelope_sha256": hashlib.sha256(
            canonical_observer_envelope(envelope)
        ).hexdigest(),
        "artifacts": list(artifacts.values()),
    }


def verify_observer_envelope(
    signed_result: dict[str, Any],
    certificate: dict[str, Any],
    qualification: dict[str, Any],
    observer_public_key: Path,
    *,
    expected_request_id: str,
    expected_host_nonce: str,
    expected_contract_sha256: str,
    expected_observer_id: str,
    expected_deployment_sha256: str,
    expected_qualification_id: str,
    expected_device_id: str,
    expected_device_boot_id: str,
    expected_firmware_elf_sha256: str,
    expected_client_spki_sha256: str,
    expected_usb_serial_number: str,
    expected_fixture_board_id: str,
    minimum_run_counter: int,
    now: datetime | None = None,
) -> dict[str, Any]:
    return _verify_observer_result(
        signed_result,
        certificate,
        qualification,
        observer_public_key,
        None,
        expected_request_id=expected_request_id,
        expected_host_nonce=expected_host_nonce,
        expected_contract_sha256=expected_contract_sha256,
        expected_observer_id=expected_observer_id,
        expected_deployment_sha256=expected_deployment_sha256,
        expected_qualification_id=expected_qualification_id,
        expected_device_id=expected_device_id,
        expected_device_boot_id=expected_device_boot_id,
        expected_firmware_elf_sha256=expected_firmware_elf_sha256,
        expected_client_spki_sha256=expected_client_spki_sha256,
        expected_usb_serial_number=expected_usb_serial_number,
        expected_fixture_board_id=expected_fixture_board_id,
        minimum_run_counter=minimum_run_counter,
        now=now,
    )


def verify_observer_result(
    signed_result: dict[str, Any],
    certificate: dict[str, Any],
    qualification: dict[str, Any],
    observer_public_key: Path,
    artifact_root: Path,
    *,
    expected_request_id: str,
    expected_host_nonce: str,
    expected_contract_sha256: str,
    expected_observer_id: str,
    expected_deployment_sha256: str,
    expected_qualification_id: str,
    expected_device_id: str,
    expected_device_boot_id: str,
    expected_firmware_elf_sha256: str,
    expected_client_spki_sha256: str,
    expected_usb_serial_number: str,
    expected_fixture_board_id: str,
    minimum_run_counter: int,
    now: datetime | None = None,
) -> dict[str, Any]:
    result = _verify_observer_result(
        signed_result,
        certificate,
        qualification,
        observer_public_key,
        artifact_root,
        expected_request_id=expected_request_id,
        expected_host_nonce=expected_host_nonce,
        expected_contract_sha256=expected_contract_sha256,
        expected_observer_id=expected_observer_id,
        expected_deployment_sha256=expected_deployment_sha256,
        expected_qualification_id=expected_qualification_id,
        expected_device_id=expected_device_id,
        expected_device_boot_id=expected_device_boot_id,
        expected_firmware_elf_sha256=expected_firmware_elf_sha256,
        expected_client_spki_sha256=expected_client_spki_sha256,
        expected_usb_serial_number=expected_usb_serial_number,
        expected_fixture_board_id=expected_fixture_board_id,
        minimum_run_counter=minimum_run_counter,
        now=now,
    )
    return {
        **{
            key: value
            for key, value in result.items()
            if key != "candidate_pass"
        },
        "passed": result["candidate_pass"],
    }
