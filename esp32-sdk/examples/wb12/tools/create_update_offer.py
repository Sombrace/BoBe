#!/usr/bin/env python3

import argparse
import base64
import hashlib
import json
import os
import re
import subprocess
import tempfile
from pathlib import Path
from urllib.parse import urlparse

MODEL = "wonderboy-wb12"
SCHEMA_VERSION = 1
MAX_OFFER_BYTES = 4096
MAX_MANIFEST_BYTES = 2048
MAX_IMAGE = 0x500000
MAX_BUILD_NUMBER = 0xFFFFFFFF
MAX_SAFE_INTEGER = 9_007_199_254_740_991
MAX_VERSION_LEN = 31
MAX_URL_LEN = 256
MAX_KEY_ID_LEN = 32
HEX64 = re.compile(r"[0-9a-f]{64}")


def regular(path: Path, name: str) -> Path:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{name} must be a regular file")
    return path


def canonical(value: dict[str, object]) -> bytes:
    return json.dumps(value, separators=(",", ":"), ensure_ascii=True).encode()


def build_offer(args: argparse.Namespace) -> dict[str, object]:
    image = regular(args.image.resolve(), "image")
    artifact = regular(args.artifact_manifest.resolve(), "artifact manifest")
    key = regular(args.private_key.resolve(), "private key")
    if args.output.exists() or args.output.is_symlink():
        raise ValueError("output must be a new path")
    metadata = json.loads(artifact.read_text())
    image_bytes = image.read_bytes()
    digest = hashlib.sha256(image_bytes).hexdigest()
    if metadata.get("image_sha256") != digest or metadata.get("image_size") != len(image_bytes):
        raise ValueError("image does not match artifact manifest")
    if metadata.get("source_dirty") is not False:
        raise ValueError("OTA offers require a clean source artifact")
    if metadata.get("target") != "esp32s3" or metadata.get("project_name") != "bobe_wb12":
        raise ValueError("artifact is not a WB-12 image")
    if not 0 < len(image_bytes) <= MAX_IMAGE:
        raise ValueError("image does not fit the OTA slot")
    if not 0 < args.build_number <= MAX_BUILD_NUMBER:
        raise ValueError("build number is outside the firmware range")
    if args.build_number <= args.current_build_number:
        raise ValueError("build number must increase")
    parsed = urlparse(args.url)
    if (
        parsed.scheme != "https"
        or not parsed.netloc
        or len(args.url.encode()) > MAX_URL_LEN
    ):
        raise ValueError("URL must be bounded HTTPS")
    if not args.key_id or len(args.key_id.encode()) > MAX_KEY_ID_LEN:
        raise ValueError("key ID is invalid")
    if (
        args.not_before < 0
        or args.expires_at <= args.not_before
        or args.expires_at > MAX_SAFE_INTEGER
    ):
        raise ValueError("validity window is invalid")
    version = str(metadata.get("app_version", ""))
    version_bytes = version.encode()
    if (
        not version_bytes
        or len(version_bytes) > MAX_VERSION_LEN
        or any(byte < 0x21 or byte > 0x7E for byte in version_bytes)
    ):
        raise ValueError("artifact version is invalid")
    manifest = {
        "schema": SCHEMA_VERSION,
        "device_model": MODEL,
        "version": version,
        "build_number": args.build_number,
        "image_size": len(image_bytes),
        "image_sha256": digest,
        "url": args.url,
        "key_id": args.key_id,
        "not_before": args.not_before,
        "expires_at": args.expires_at,
    }
    manifest_bytes = canonical(manifest)
    if len(manifest_bytes) > MAX_MANIFEST_BYTES:
        raise ValueError("canonical manifest exceeds the BodyLink limit")
    with tempfile.NamedTemporaryFile() as manifest_file, tempfile.NamedTemporaryFile() as signature_file:
        manifest_file.write(manifest_bytes)
        manifest_file.flush()
        result = subprocess.run(
            ["openssl", "dgst", "-sha256", "-sign", str(key), "-out", signature_file.name, manifest_file.name],
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise ValueError(f"OpenSSL signing failed: {result.stderr.strip()}")
        signature = base64.b64encode(Path(signature_file.name).read_bytes()).decode()
    offer = {
        "type": "update.offer",
        "protocol_major": 1,
        "manifest": manifest_bytes.decode(),
        "signature": signature,
    }
    offer_bytes = canonical(offer) + b"\n"
    if len(offer_bytes) > MAX_OFFER_BYTES:
        raise ValueError("canonical offer exceeds the BodyLink control limit")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as output:
        output.write(offer_bytes)
        output.flush()
        os.fsync(output.fileno())
    return {"output": str(args.output), "manifest_sha256": hashlib.sha256(manifest_bytes).hexdigest(), "image_sha256": digest}


def parser() -> argparse.ArgumentParser:
    value = argparse.ArgumentParser(description="Create a canonical signed WB-12 BodyLink OTA offer.")
    value.add_argument("--image", required=True, type=Path)
    value.add_argument("--artifact-manifest", required=True, type=Path)
    value.add_argument("--private-key", required=True, type=Path)
    value.add_argument("--output", required=True, type=Path)
    value.add_argument("--url", required=True)
    value.add_argument("--key-id", required=True)
    value.add_argument("--build-number", required=True, type=int)
    value.add_argument("--current-build-number", required=True, type=int)
    value.add_argument("--not-before", required=True, type=int)
    value.add_argument("--expires-at", required=True, type=int)
    return value


def main() -> int:
    try:
        result = build_offer(parser().parse_args())
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(str(error)) from error
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
