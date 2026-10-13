import argparse
import base64
import binascii
import hashlib
import json
import os
import struct
import subprocess
import sys
from datetime import UTC, datetime
from pathlib import Path
from typing import cast

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import (
    Ed25519PrivateKey,
    Ed25519PublicKey,
)
from esptool.bin_image import ESP32S3FirmwareImage, LoadFirmwareImage

from bobe_hil.paths import git_metadata, sha256_file


APP_DESC = struct.Struct("<II8s32s32s16s16s32s32sHHB3s72s")
OTA_ENTRY = struct.Struct("<I20xII")
OTA_ENTRY_OFFSETS = (0, 0x1000)
VALID_OTA_STATES = {
    0: "new",
    1: "pending_verify",
    2: "valid",
}
APP_PROJECT = "bobe_wb12"
APP_TARGET = "esp32s3"
APP_PARTITION_SIZE = 0x500000
MANIFEST_SCHEMA = 1
PROFILES = {"fallback", "ptt", "edge"}
REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
SIGNING_PUBLIC_KEY = (
    REPOSITORY_ROOT / "hil" / "security" / "manifest-signing-public.pem"
)
SIGNATURE_ALGORITHM = "ed25519"


def checked_file(path: Path, description: str) -> Path:
    if path.is_symlink():
        raise ValueError(f"{description} must not be a symbolic link")
    if not path.is_file():
        raise ValueError(f"{description} does not exist")
    return path


def decoded_string(value: bytes, field: str) -> str:
    try:
        return value.split(b"\0", 1)[0].decode("utf-8")
    except UnicodeDecodeError as error:
        raise ValueError(f"{field} is not valid UTF-8") from error


def inspect_application(path: Path) -> dict[str, object]:
    checked_file(path, "application image")
    image_size = path.stat().st_size
    if image_size <= 0 or image_size > APP_PARTITION_SIZE:
        raise ValueError("application image does not fit the 5 MiB OTA slot")

    data = path.read_bytes()
    try:
        image = LoadFirmwareImage(APP_TARGET, data)
    except Exception as error:
        raise ValueError("application is not a valid ESP32-S3 image") from error

    if not isinstance(image, ESP32S3FirmwareImage):
        raise ValueError("application image is not for ESP32-S3")
    if image.chip_id != image.ROM_LOADER.IMAGE_CHIP_ID:
        raise ValueError("application chip identifier is not ESP32-S3")
    if image.checksum != image.calculate_checksum():
        raise ValueError("application checksum is invalid")
    if not image.append_digest or image.stored_digest != image.calc_digest:
        raise ValueError("application validation digest is missing or invalid")

    descriptor_segment = next(
        (
            segment.data
            for segment in image.segments
            if "DROM" in segment.get_memory_type(image)
        ),
        None,
    )
    if descriptor_segment is None or len(descriptor_segment) < APP_DESC.size:
        raise ValueError("application description is missing")

    unpacked = APP_DESC.unpack_from(descriptor_segment)
    magic_word = unpacked[0]
    if magic_word != 0xABCD5432:
        raise ValueError("application description magic is invalid")

    project_name = decoded_string(unpacked[4], "project name")
    if project_name != APP_PROJECT:
        raise ValueError(
            f"unexpected application project {project_name!r}; expected {APP_PROJECT!r}"
        )

    return {
        "schema_version": MANIFEST_SCHEMA,
        "target": APP_TARGET,
        "project_name": project_name,
        "app_version": decoded_string(unpacked[3], "app version"),
        "compile_time": decoded_string(unpacked[5], "compile time"),
        "compile_date": decoded_string(unpacked[6], "compile date"),
        "idf_version": decoded_string(unpacked[7], "ESP-IDF version"),
        "app_elf_sha256": unpacked[8].hex(),
        "secure_version": unpacked[1],
        "image_size": image_size,
        "image_sha256": hashlib.sha256(data).hexdigest(),
    }


def create_manifest(
    root: Path,
    image_path: Path,
    elf_path: Path,
    profile: str,
) -> dict[str, object]:
    if profile not in PROFILES:
        raise ValueError(f"unsupported firmware profile {profile!r}")
    checked_file(elf_path, "ELF file")
    manifest = inspect_application(image_path)
    commit, dirty = git_metadata(root)
    manifest.update(
        {
            "profile": profile,
            "elf_file_sha256": sha256_file(elf_path),
            "source_commit": commit,
            "source_dirty": dirty,
            "created_at": datetime.now(UTC).isoformat(timespec="seconds"),
        }
    )
    return manifest


def manifest_payload(manifest: dict[str, object]) -> bytes:
    unsigned = {
        key: value
        for key, value in manifest.items()
        if key not in {"signature", "signature_algorithm"}
    }
    return json.dumps(
        unsigned,
        separators=(",", ":"),
        sort_keys=True,
    ).encode()


def sign_manifest(
    manifest: dict[str, object],
    private_key: Path,
) -> dict[str, object]:
    checked_file(private_key, "manifest signing key")
    try:
        loaded = serialization.load_pem_private_key(
            private_key.read_bytes(),
            password=None,
        )
    except (TypeError, ValueError) as error:
        raise ValueError("manifest signing key is invalid") from error
    if not isinstance(loaded, Ed25519PrivateKey):
        raise ValueError("manifest signing key is not Ed25519")
    signature = base64.b64encode(
        loaded.sign(manifest_payload(manifest))
    ).decode()
    return {
        **manifest,
        "signature_algorithm": SIGNATURE_ALGORITHM,
        "signature": signature,
    }


def verify_manifest_signature(
    manifest: dict[str, object],
    public_key: Path = SIGNING_PUBLIC_KEY,
) -> None:
    checked_file(public_key, "manifest signing public key")
    if manifest.get("signature_algorithm") != SIGNATURE_ALGORITHM:
        raise ValueError("application manifest signature algorithm is invalid")
    encoded = manifest.get("signature")
    if not isinstance(encoded, str):
        raise ValueError("application manifest signature is missing")
    try:
        signature = base64.b64decode(encoded, validate=True)
    except (ValueError, binascii.Error) as error:
        raise ValueError("application manifest signature is malformed") from error
    try:
        loaded = serialization.load_pem_public_key(public_key.read_bytes())
    except ValueError as error:
        raise ValueError("manifest signing public key is invalid") from error
    if not isinstance(loaded, Ed25519PublicKey):
        raise ValueError("manifest signing public key is not Ed25519")
    try:
        loaded.verify(signature, manifest_payload(manifest))
    except InvalidSignature as error:
        raise ValueError("application manifest signature is invalid") from error


def load_manifest(
    path: Path,
    image_path: Path,
    *,
    require_signature: bool = False,
) -> dict[str, object]:
    checked_file(path, "application manifest")
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ValueError("application manifest is not valid JSON") from error
    if not isinstance(manifest, dict):
        raise ValueError("application manifest must be a JSON object")

    actual = inspect_application(image_path)
    required = {
        "schema_version",
        "target",
        "project_name",
        "app_version",
        "idf_version",
        "app_elf_sha256",
        "image_size",
        "image_sha256",
        "profile",
        "elf_file_sha256",
        "source_commit",
        "source_dirty",
        "created_at",
    }
    missing = sorted(required - manifest.keys())
    if missing:
        raise ValueError(f"application manifest is missing: {', '.join(missing)}")
    for key in (
        "schema_version",
        "target",
        "project_name",
        "app_version",
        "idf_version",
        "app_elf_sha256",
        "image_size",
        "image_sha256",
    ):
        if manifest[key] != actual[key]:
            raise ValueError(f"application manifest {key} does not match image")
    if manifest["profile"] not in PROFILES:
        raise ValueError("application manifest profile is unsupported")
    if (
        not isinstance(manifest["source_commit"], str)
        or len(manifest["source_commit"]) != 40
    ):
        raise ValueError("application manifest source commit is malformed")
    if not isinstance(manifest["source_dirty"], bool):
        raise ValueError("application manifest dirty state is malformed")
    if require_signature:
        verify_manifest_signature(manifest)
    return manifest


def parse_otadata(data: bytes, ota_partition_count: int = 2) -> dict[str, object]:
    if ota_partition_count <= 0:
        raise ValueError("OTA partition count must be positive")
    if len(data) < OTA_ENTRY_OFFSETS[-1] + OTA_ENTRY.size:
        raise ValueError("otadata partition is truncated")

    entries: list[dict[str, object]] = []
    selectable_entries: list[dict[str, object]] = []
    for copy_index, offset in enumerate(OTA_ENTRY_OFFSETS):
        sequence, state, stored_crc = OTA_ENTRY.unpack_from(data, offset)
        calculated_crc = (
            binascii.crc32(struct.pack("<I", sequence), 0xFFFFFFFF) & 0xFFFFFFFF
        )
        valid = sequence != 0xFFFFFFFF and stored_crc == calculated_crc
        entry: dict[str, object] = {
            "copy": copy_index,
            "sequence": sequence,
            "state": state,
            "state_name": VALID_OTA_STATES.get(state, "unknown"),
            "stored_crc": stored_crc,
            "calculated_crc": calculated_crc,
            "valid": valid,
            "ota_slot": (
                (sequence - 1) % ota_partition_count if valid else None
            ),
        }
        entries.append(entry)
        if valid and state in VALID_OTA_STATES:
            selectable_entries.append(entry)

    if not selectable_entries:
        raise ValueError("otadata has no valid selection entry")
    selected = max(
        selectable_entries,
        key=lambda entry: cast(int, entry["sequence"]),
    )
    return {
        "schema_version": MANIFEST_SCHEMA,
        "entries": entries,
        "selected_copy": selected["copy"],
        "selected_sequence": selected["sequence"],
        "selected_state": selected["state_name"],
        "selected_slot": selected["ota_slot"],
        "selected_partition": f"ota_{selected['ota_slot']}",
    }


def write_json(path: Path | None, value: dict[str, object]) -> None:
    rendered = json.dumps(value, indent=2, sort_keys=True) + "\n"
    if path is None:
        sys.stdout.write(rendered)
        return
    if path.is_symlink():
        raise ValueError("output must not be a symbolic link")
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        output.write(rendered)


def main() -> None:
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(
        description="Inspect and authorize BoBe application artifacts."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    inspect_parser = subparsers.add_parser("inspect-app")
    inspect_parser.add_argument("image", type=Path)
    inspect_parser.add_argument("--manifest", type=Path)
    inspect_parser.add_argument("--require-clean", action="store_true")
    inspect_parser.add_argument("--require-signature", action="store_true")

    manifest_parser = subparsers.add_parser("create-manifest")
    manifest_parser.add_argument("image", type=Path)
    manifest_parser.add_argument("elf", type=Path)
    manifest_parser.add_argument(
        "--profile",
        required=True,
        choices=sorted(PROFILES),
    )
    manifest_parser.add_argument("--output", required=True, type=Path)

    ota_parser = subparsers.add_parser("inspect-otadata")
    ota_parser.add_argument("image", type=Path)
    ota_parser.add_argument("--require-slot", type=int, choices=(0, 1))

    args = parser.parse_args()
    try:
        if args.command == "inspect-app":
            result = inspect_application(args.image)
            if args.manifest is not None:
                manifest = load_manifest(
                    args.manifest,
                    args.image,
                    require_signature=args.require_signature,
                )
                if args.require_clean and manifest["source_dirty"]:
                    raise ValueError(
                        "application manifest was created from a dirty worktree"
                    )
                result["manifest"] = manifest
            write_json(None, result)
        elif args.command == "create-manifest":
            write_json(
                args.output,
                create_manifest(root, args.image, args.elf, args.profile),
            )
        else:
            checked_file(args.image, "otadata image")
            result = parse_otadata(args.image.read_bytes())
            if (
                args.require_slot is not None
                and result["selected_slot"] != args.require_slot
            ):
                raise ValueError(
                    "selected OTA partition is "
                    f"{result['selected_partition']}, expected ota_{args.require_slot}"
                )
            write_json(None, result)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
