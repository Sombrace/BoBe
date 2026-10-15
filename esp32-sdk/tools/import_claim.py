#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import shutil
import stat
import subprocess
from pathlib import Path
from urllib.parse import urlparse

from cryptography import x509
from cryptography.hazmat.primitives import serialization

FIELDS = {"device_id", "endpoint_uri", "trust_kid", "controller_epoch", "server_ca_pem", "client_cert_pem", "client_key_pem"}


def unique_fields(pairs: list[tuple[str, object]]) -> dict:
    value = {}
    for name, item in pairs:
        if name in value:
            raise ValueError("claim contains duplicate fields")
        value[name] = item
    return value


def read_claim(path: Path) -> dict:
    if path.is_symlink() or not path.is_file():
        raise ValueError("claim must be a regular file")
    metadata = path.stat()
    if stat.S_IMODE(metadata.st_mode) & 0o077:
        raise ValueError("claim must be readable only by its owner (chmod 600)")
    if not 0 < metadata.st_size <= 32768:
        raise ValueError("claim size is invalid")
    value = json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=unique_fields)
    if not isinstance(value, dict) or set(value) != FIELDS:
        raise ValueError("claim fields do not match the BodyLink provisioning format")
    for field in FIELDS - {"controller_epoch"}:
        if not isinstance(value[field], str) or not value[field] or "\0" in value[field]:
            raise ValueError(f"claim {field} is invalid")
    if len(value["device_id"].encode()) > 63:
        raise ValueError("device ID is too long")
    endpoint = value["endpoint_uri"]
    if not endpoint.isascii() or any(character.isspace() or ord(character) < 32 for character in endpoint):
        raise ValueError("endpoint must be an ASCII URL without whitespace or control characters")
    parsed = urlparse(endpoint)
    port = parsed.port
    if (parsed.scheme != "wss" or not parsed.hostname or parsed.username or parsed.password
            or parsed.fragment or parsed.query or port == 0 or len(endpoint.encode()) >= 256):
        raise ValueError("endpoint must be a bounded WSS URL without embedded credentials, query, or fragment")
    if type(value["controller_epoch"]) is not int or not 1 <= value["controller_epoch"] <= 9007199254740991:
        raise ValueError("controller epoch must be a positive JSON-safe integer")
    for field in ("server_ca_pem", "client_cert_pem", "client_key_pem"):
        if not 1 < len(value[field].encode()) < 4096:
            raise ValueError(f"claim {field} is too large")
    if "-----BEGIN CERTIFICATE-----" not in value["server_ca_pem"] or "-----BEGIN CERTIFICATE-----" not in value["client_cert_pem"]:
        raise ValueError("claim certificate PEM is invalid")
    if "PRIVATE KEY-----" not in value["client_key_pem"]:
        raise ValueError("claim private-key PEM is invalid")
    authorities = x509.load_pem_x509_certificates(value["server_ca_pem"].encode())
    certificates = x509.load_pem_x509_certificates(value["client_cert_pem"].encode())
    if not authorities or not certificates:
        raise ValueError("claim certificate chain is empty")
    key = serialization.load_pem_private_key(value["client_key_pem"].encode(), password=None)
    public_format = serialization.PublicFormat.SubjectPublicKeyInfo
    if key.public_key().public_bytes(serialization.Encoding.DER, public_format) != certificates[0].public_key().public_bytes(serialization.Encoding.DER, public_format):
        raise ValueError("claim certificate and private key do not match")
    return value


def import_claim(claim_path: Path, project: Path) -> Path:
    claim = read_claim(claim_path)
    project = project.expanduser().resolve()
    if not (project / "CMakeLists.txt").is_file() or not (project / "main").is_dir():
        raise ValueError("destination is not an ESP-IDF application")
    ignore = project / ".gitignore"
    if ignore.is_symlink() or (ignore.exists() and not ignore.is_file()):
        raise ValueError("project .gitignore must be a regular file")
    private = project / "private"
    if private.is_symlink():
        raise ValueError("project private directory must not be a symbolic link")
    private.mkdir(mode=0o700, exist_ok=True)
    if stat.S_IMODE(private.stat().st_mode) & 0o077:
        raise ValueError("project private directory must have mode 700")
    output = private / "bobe_credentials.h"
    if output.exists() or output.is_symlink():
        raise ValueError("project already contains credentials; they will not be overwritten")
    if shutil.which("git") is not None:
        tracked = subprocess.run(
            ["git", "-C", str(project), "ls-files", "--error-unmatch", "--", "private/bobe_credentials.h"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False,
        )
        if tracked.returncode == 0:
            raise ValueError("credential path is tracked by Git; remove it from the index first")
    text = ignore.read_text(encoding="utf-8") if ignore.exists() else ""
    if not text.rstrip().endswith("\n/private/") and text.strip() != "/private/":
        with ignore.open("a", encoding="utf-8") as destination:
            destination.write("\n/private/\n")
    body = (
        '#pragma once\n#include "esp_bobe_client.h"\n\n'
        f'#define BOBE_PROVISIONED_DEVICE_ID {json.dumps(claim["device_id"])}\n'
        'static const esp_bobe_credentials_t bobe_credentials = {\n'
        f'    .endpoint = {json.dumps(claim["endpoint_uri"])},\n'
        f'    .server_ca = {json.dumps(claim["server_ca_pem"])},\n'
        f'    .client_certificate = {json.dumps(claim["client_cert_pem"])},\n'
        f'    .client_key = {json.dumps(claim["client_key_pem"])},\n'
        f'    .controller_epoch = UINT64_C({claim["controller_epoch"]}),\n'
        '};\n'
    )
    descriptor = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as destination:
            destination.write(body)
    except OSError:
        output.unlink(missing_ok=True)
        raise
    return output


def main() -> None:
    parser = argparse.ArgumentParser(description="Import an owner-issued BodyLink claim into one project's ignored private directory.")
    parser.add_argument("claim", type=Path)
    parser.add_argument("--project", type=Path, required=True)
    args = parser.parse_args()
    try:
        path = import_claim(args.claim, args.project)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    print(f"Imported private credentials into {path}; rebuild the application")
    print("The owner must separately enroll this certificate and grant the requested minor-2 capabilities")


if __name__ == "__main__":
    main()
