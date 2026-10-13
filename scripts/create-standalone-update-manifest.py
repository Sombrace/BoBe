#!/usr/bin/env python3

import argparse
import hashlib
import json
import pathlib
import tarfile

PLATFORMS = {
    "darwin-arm64",
    "darwin-x86_64",
    "linux-x86_64",
    "linux-aarch64",
}
REQUIRED_COMPATIBILITY = {
    "schema",
    "version",
    "platform",
    "database_schema",
    "attach_protocol_min",
    "attach_protocol_max",
}
REQUIRED_RUNTIME_FILES = {
    "bin/bobe",
    "bin/node",
    "bin/agent/dist/main.js",
    "bin/agent/assets/chat.md",
    "bin/agent/fixtures/host-contract-v1.json",
    "bin/agent/package.json",
    "bin/agent/licenses/node-LICENSE",
    "bin/agent/licenses/rg-MIT",
    "bin/agent/licenses/fd-MIT",
    "bin/agent/bin/rg",
    "bin/agent/bin/fd",
    "bin/agent/node_modules/@earendil-works/pi-coding-agent/package.json",
    "bin/agent/node_modules/@earendil-works/pi-ai/package.json",
    "bin/agent/node_modules/@earendil-works/pi-agent-core/package.json",
    "bin/agent/node_modules/quickjs-wasi/quickjs.wasm",
    "bin/agent/node_modules/@silvia-odwyer/photon-node/photon_rs_bg.wasm",
}


def fail(message: str) -> None:
    raise SystemExit(message)


def safe_parts(name: str) -> tuple[str, ...]:
    path = pathlib.PurePosixPath(name)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        fail(f"unsafe archive path: {name}")
    return path.parts


def inspect_archive(archive: pathlib.Path) -> dict[str, object]:
    if not archive.is_file():
        fail(f"standalone archive not found: {archive}")
    try:
        with tarfile.open(archive, "r:gz") as bundle:
            members = bundle.getmembers()
            if not members:
                fail("standalone archive is empty")
            roots: set[str] = set()
            compatibility_member = None
            runtime_files: set[str] = set()
            for member in members:
                if member.type in {
                    tarfile.XHDTYPE,
                    tarfile.XGLTYPE,
                    tarfile.GNUTYPE_LONGNAME,
                    tarfile.GNUTYPE_LONGLINK,
                }:
                    continue
                parts = safe_parts(member.name)
                roots.add(parts[0])
                if not (member.isfile() or member.isdir()):
                    fail(f"unsupported archive entry: {member.name}")
                if member.isfile() and member.size > 0:
                    runtime_files.add("/".join(parts[1:]))
                if len(parts) == 2 and parts[1] == "compatibility.json":
                    compatibility_member = member
            if len(roots) != 1:
                fail("standalone archive must contain one root directory")
            if compatibility_member is None:
                fail("standalone archive has no root compatibility.json")
            missing_runtime = REQUIRED_RUNTIME_FILES - runtime_files
            if missing_runtime:
                fail(f"standalone archive is missing runtime files: {', '.join(sorted(missing_runtime))}")
            source = bundle.extractfile(compatibility_member)
            if source is None:
                fail("standalone compatibility.json is not a regular file")
            compatibility = json.load(source)
    except (tarfile.TarError, json.JSONDecodeError) as error:
        fail(f"invalid standalone archive: {error}")

    if not isinstance(compatibility, dict):
        fail("standalone compatibility.json must be an object")
    missing = REQUIRED_COMPATIBILITY - compatibility.keys()
    if missing:
        fail(f"standalone compatibility.json is missing: {', '.join(sorted(missing))}")
    if compatibility["schema"] != 1:
        fail("unsupported standalone compatibility schema")
    if compatibility["platform"] not in PLATFORMS:
        fail(f"unsupported standalone platform: {compatibility['platform']}")
    if not isinstance(compatibility["version"], str) or not compatibility["version"]:
        fail("standalone version must be a non-empty string")
    for field in ("database_schema", "attach_protocol_min", "attach_protocol_max"):
        if not isinstance(compatibility[field], int):
            fail(f"{field} must be an integer")
    if compatibility["attach_protocol_min"] > compatibility["attach_protocol_max"]:
        fail("attach_protocol_min must not exceed attach_protocol_max")
    return compatibility


def create_manifest(archive: pathlib.Path) -> dict[str, object]:
    compatibility = inspect_archive(archive)
    digest = hashlib.sha256()
    with archive.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return {
        "schema": 1,
        "version": compatibility["version"],
        "platform": compatibility["platform"],
        "database_schema": compatibility["database_schema"],
        "attach_protocol_min": compatibility["attach_protocol_min"],
        "attach_protocol_max": compatibility["attach_protocol_max"],
        "archive_sha256": digest.hexdigest(),
        "archive": archive.name,
    }


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Create the canonical signed-update manifest for a standalone BoBe archive."
    )
    parser.add_argument("archive", type=pathlib.Path)
    parser.add_argument("--output", type=pathlib.Path)
    args = parser.parse_args()

    archive = args.archive.resolve()
    output = args.output or archive.with_name(f"{archive.name}.manifest.json")
    output = output.resolve()
    manifest = create_manifest(archive)
    encoded = json.dumps(manifest, separators=(",", ":"), ensure_ascii=True).encode()
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(f".{output.name}.tmp")
    temporary.write_bytes(encoded)
    temporary.replace(output)
    print(output)


if __name__ == "__main__":
    main()
