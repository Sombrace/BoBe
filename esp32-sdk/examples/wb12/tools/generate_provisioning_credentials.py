#!/usr/bin/env python3

import argparse
import json
import os
import secrets
import shutil
import stat
import sys
import tempfile
import uuid
from datetime import UTC, datetime
from pathlib import Path


def c_array(data: bytes) -> str:
    values = [f"0x{value:02x}" for value in data]
    return "\n".join(
        "    " + ", ".join(values[index : index + 12]) + ","
        for index in range(0, len(values), 12)
    )


def private_output(root: Path, requested: Path) -> Path:
    private_root = root / "private"
    if private_root.is_symlink():
        raise ValueError("private output root must not be a symbolic link")
    output = requested if requested.is_absolute() else root / requested
    resolved = output.resolve()
    resolved_private = private_root.resolve()
    if resolved != resolved_private and resolved_private not in resolved.parents:
        raise ValueError("credentials must be written under the project private directory")
    return resolved


def write_private(path: Path, content: str) -> None:
    descriptor = os.open(
        path,
        os.O_CREAT | os.O_EXCL | os.O_WRONLY,
        stat.S_IRUSR | stat.S_IWUSR,
    )
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        output.write(content)
        output.flush()
        os.fsync(output.fileno())


def fsync_directory(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def publish_bundle(staged: Path, output: Path, *, rotate: bool) -> Path | None:
    if output.is_symlink():
        raise ValueError("credential output must not be a symbolic link")
    if rotate:
        if not output.is_dir():
            raise ValueError("--rotate requires an existing credential bundle")
    elif output.exists():
        raise ValueError("--create refuses to replace existing credentials")

    archived: Path | None = None
    if rotate:
        manifest = output / "security2-private.json"
        if manifest.is_symlink() or not manifest.is_file():
            raise ValueError("existing credential manifest is unavailable")
        manifest_bytes = manifest.read_bytes()
        value = json.loads(manifest_bytes)
        generation = value.get("generation_id")
        if (
            not isinstance(generation, str)
            or len(generation) != 32
            or any(character not in "0123456789abcdef" for character in generation)
        ):
            raise ValueError("existing credential manifest has no generation id")
        history = output.parent / f"{output.name}-history"
        if history.is_symlink():
            raise ValueError("credential history must not be a symbolic link")
        history.mkdir(mode=0o700, exist_ok=True)
        history.chmod(0o700)
        archived = history / generation
        if archived.exists() or archived.is_symlink():
            raise ValueError("credential generation is already archived")
        output.replace(archived)
        try:
            staged.replace(output)
        except OSError:
            archived.replace(output)
            raise
        fsync_directory(history)
    else:
        staged.replace(output)
    fsync_directory(output.parent)
    return archived


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate private per-device Protocomm Security 2 credentials."
    )
    parser.add_argument(
        "--device-id",
        required=True,
        help="Canonical stable BodyLink device UUID.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path("private/provisioning"),
        help="Private bundle directory (default: private/provisioning).",
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--create",
        action="store_true",
        help="Create a new bundle and fail if credentials already exist.",
    )
    mode.add_argument(
        "--rotate",
        action="store_true",
        help="Archive the current bundle and publish one replacement.",
    )
    args = parser.parse_args()

    root = Path(__file__).resolve().parent.parent
    tool = (
        root
        / "managed_components/espressif__network_provisioning/tool/"
        "esp_prov/esp_prov.py"
    )
    if not tool.is_file():
        parser.error("run tools/bootstrap_idf.sh and tools/idf.sh reconfigure first")

    try:
        device_id = str(uuid.UUID(args.device_id))
    except ValueError as error:
        parser.error(f"--device-id must be a canonical UUID: {error}")
    if device_id != args.device_id:
        parser.error("--device-id must use canonical UUID formatting")

    provisioning_tool = tool.parent
    security_tool = provisioning_tool / "security"
    sys.path.insert(0, str(provisioning_tool))
    sys.path.insert(0, str(security_tool))
    from srp6a import generate_salt_and_verifier

    username = f"bobe-{device_id}"
    password = "B" + secrets.token_urlsafe(24)
    softap_password = "S" + secrets.token_urlsafe(18)
    salt, verifier = generate_salt_and_verifier(
        username,
        password,
        len_s=16,
    )
    if len(salt) != 16 or len(verifier) != 384:
        raise RuntimeError("Espressif SRP generator returned unexpected lengths")

    try:
        output = private_output(root, args.output)
    except ValueError as error:
        parser.error(str(error))
    (root / "private").mkdir(parents=False, exist_ok=True, mode=0o700)
    (root / "private").chmod(stat.S_IRWXU)
    output.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    output.parent.chmod(stat.S_IRWXU)
    generation_id = secrets.token_hex(16)
    source_content = (
        '#include "bobe_provisioning_credentials.h"\n\n'
        f"static const uint8_t sec2_salt[] = {{\n{c_array(salt)}\n}};\n\n"
        f"static const uint8_t sec2_verifier[] = {{\n{c_array(verifier)}\n}};\n\n"
        "esp_err_t bobe_provisioning_credentials_get(\n"
        "    bobe_provisioning_credentials_t *credentials\n"
        ")\n{\n"
        "    if (credentials == NULL) {\n"
        "        return ESP_ERR_INVALID_ARG;\n"
        "    }\n"
        f'    credentials->username = "{username}";\n'
        f'    credentials->softap_password = "{softap_password}";\n'
        "    credentials->salt = sec2_salt;\n"
        "    credentials->salt_length = sizeof(sec2_salt);\n"
        "    credentials->verifier = sec2_verifier;\n"
        "    credentials->verifier_length = sizeof(sec2_verifier);\n"
        "    return ESP_OK;\n"
        "}\n"
    )
    manifest_content = (
        json.dumps(
            {
                "schema_version": 1,
                "generation_id": generation_id,
                "created_at": datetime.now(UTC).isoformat(
                    timespec="seconds"
                ),
                "device_id": device_id,
                "username": username,
                "password": password,
                "softap_password": softap_password,
                "qr_payload": {
                    "ver": "v1",
                    "name": "derived-at-runtime-from-device-mac",
                    "username": username,
                    "pop": password,
                    "transport": "softap",
                },
            },
            indent=2,
        )
        + "\n"
    )
    staged = Path(
        tempfile.mkdtemp(
            prefix=f".{output.name}.",
            dir=output.parent,
        )
    )
    staged.chmod(0o700)
    archived: Path | None = None
    try:
        write_private(
            staged / "bobe_provisioning_credentials_private.c",
            source_content,
        )
        write_private(
            staged / "security2-private.json",
            manifest_content,
        )
        fsync_directory(staged)
        archived = publish_bundle(staged, output, rotate=args.rotate)
    finally:
        if staged.exists():
            shutil.rmtree(staged)
    print(
        json.dumps(
            {
                "schema_version": 1,
                "generation_id": generation_id,
                "device_id": device_id,
                "output": str(output),
                "operation": "rotate" if args.rotate else "create",
                "archived": str(archived) if archived is not None else None,
            },
            indent=2,
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
