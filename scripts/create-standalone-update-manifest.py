#!/usr/bin/env python3

import argparse
import hashlib
import json
import pathlib
import re
import tarfile

PLATFORM_EXECUTABLES = {
    "darwin-arm64": ("Mach-O arm64", "Mach-O", 0x0100000C),
    "darwin-x86_64": ("Mach-O x86_64", "Mach-O", 0x01000007),
    "linux-x86_64": ("ELF x86-64", "ELF", 62),
    "linux-aarch64": ("ELF AArch64", "ELF", 183),
}
EXECUTABLES = ("bin/bobe", "bin/node")
REQUIRED_COMPATIBILITY = {
    "schema",
    "version",
    "platform",
    "database_schema",
    "database_schema_min",
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
LINUX_RUNTIME_FILES = {
    "bin/agent/lib/libatomic.so.1",
    "bin/agent/licenses/libatomic-LICENSE",
}


def fail(message: str) -> None:
    raise SystemExit(message)


def safe_parts(name: str) -> tuple[str, ...]:
    path = pathlib.PurePosixPath(name)
    if path.is_absolute() or any(part in {"", ".", ".."} for part in path.parts):
        fail(f"unsafe archive path: {name}")
    return path.parts


def executable_architectures(header: bytes) -> tuple[str, set[int]]:
    if header[:4] == b"\x7fELF" and len(header) >= 20:
        if header[4:6] != b"\x02\x01":
            return "ELF", set()
        return "ELF", {int.from_bytes(header[18:20], "little")}
    if header[:4] == b"\xcf\xfa\xed\xfe" and len(header) >= 8:
        return "Mach-O", {int.from_bytes(header[4:8], "little")}
    if header[:4] in {b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf"} and len(header) >= 8:
        entry_size = 20 if header[3] == 0xBE else 32
        count = int.from_bytes(header[4:8], "big")
        if count == 0 or len(header) < 8 + count * entry_size:
            return "Mach-O", set()
        return "Mach-O", {
            int.from_bytes(header[8 + index * entry_size : 12 + index * entry_size], "big")
            for index in range(count)
        }
    return "unknown", set()


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
            executable_headers: dict[str, bytes] = {}
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
                relative = "/".join(parts[1:])
                if member.isfile() and member.size > 0:
                    runtime_files.add(relative)
                    if relative in EXECUTABLES:
                        source = bundle.extractfile(member)
                        if source is None:
                            fail(f"standalone {member.name} is not a regular file")
                        executable_headers[relative] = source.read(4096)
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
    if compatibility["platform"] not in PLATFORM_EXECUTABLES:
        fail(f"unsupported standalone platform: {compatibility['platform']}")
    if compatibility["platform"].startswith("linux-"):
        missing_runtime = LINUX_RUNTIME_FILES - runtime_files
        if missing_runtime:
            fail(f"standalone archive is missing runtime files: {', '.join(sorted(missing_runtime))}")
    if not isinstance(compatibility["version"], str) or not compatibility["version"]:
        fail("standalone version must be a non-empty string")
    for field in (
        "database_schema",
        "database_schema_min",
        "attach_protocol_min",
        "attach_protocol_max",
    ):
        if not isinstance(compatibility[field], int):
            fail(f"{field} must be an integer")
    if compatibility["database_schema_min"] > compatibility["database_schema"]:
        fail("database_schema_min must not exceed database_schema")
    if compatibility["attach_protocol_min"] > compatibility["attach_protocol_max"]:
        fail("attach_protocol_min must not exceed attach_protocol_max")
    expected_root = f"bobe-daemon-{compatibility['version']}-{compatibility['platform']}"
    root = next(iter(roots))
    if root != expected_root:
        fail(f"standalone archive root {root} does not match compatibility.json ({expected_root})")
    if archive.name != f"{expected_root}.tar.gz":
        fail(f"standalone archive name {archive.name} does not match compatibility.json ({expected_root}.tar.gz)")
    description, binary_format, machine = PLATFORM_EXECUTABLES[compatibility["platform"]]
    for name in EXECUTABLES:
        found_format, machines = executable_architectures(executable_headers[name])
        if found_format != binary_format or machine not in machines:
            fail(f"standalone {name} is not built for {compatibility['platform']} ({description} expected)")
    return compatibility


def verify_checksum(archive: pathlib.Path, digest: str) -> None:
    checksum = archive.with_name(f"{archive.name}.sha256")
    if not checksum.is_file():
        fail(f"standalone archive checksum not found: {checksum}")
    lines = checksum.read_text(encoding="utf-8").splitlines()
    match = re.fullmatch(r"([0-9A-Fa-f]{64}) [ *](.+)", lines[0]) if len(lines) == 1 else None
    if match is None:
        fail(f"{checksum.name} must contain one '<sha256>  {archive.name}' line")
    if match.group(2) != archive.name:
        fail(f"{checksum.name} describes {match.group(2)}, not {archive.name}")
    if match.group(1).lower() != digest:
        fail(f"{archive.name} does not match the SHA-256 in {checksum.name}")


def create_manifest(archive: pathlib.Path) -> dict[str, object]:
    if not archive.is_file():
        fail(f"standalone archive not found: {archive}")
    digest = hashlib.sha256()
    with archive.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    verify_checksum(archive, digest.hexdigest())
    compatibility = inspect_archive(archive)
    return {
        "schema": 1,
        "version": compatibility["version"],
        "platform": compatibility["platform"],
        "database_schema": compatibility["database_schema"],
        "database_schema_min": compatibility["database_schema_min"],
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
