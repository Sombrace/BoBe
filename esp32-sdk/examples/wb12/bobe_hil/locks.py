import argparse
import json
import os
import re
import sys
from datetime import UTC, datetime
from pathlib import Path

from bobe_hil.image import (
    PROFILES,
    inspect_application,
    sha256_file,
    write_json,
)


LKG_LOCK_KEYS = {
    "LKG_SOURCE_COMMIT",
    "LKG_IMAGE_SHA256",
    "LKG_ELF_SHA256",
    "LKG_APP_ELF_SHA256",
    "LKG_IMAGE_SIZE",
    "LKG_BUILD_DIRECTORY",
}
SNAPSHOT_LOCK_KEYS = {
    "SNAPSHOT_SOURCE_COMMIT",
    "SNAPSHOT_IMAGE_SHA256",
    "SNAPSHOT_ELF_SHA256",
    "SNAPSHOT_APP_ELF_SHA256",
    "SNAPSHOT_IMAGE_SIZE",
    "SNAPSHOT_BUILD_DIRECTORY",
}
SNAPSHOT_BUILD_DIRECTORY = "build-bobe-snapshot"


def parse_lock(path: Path, prefix: str, keys: set[str], description: str) -> dict[str, str]:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{description} is unavailable or is not a regular file")
    values: dict[str, str] = {}
    for line in path.read_text(encoding="ascii").splitlines():
        if not line or line.count("=") != 1:
            raise ValueError(f"{description} contains a malformed line")
        key, value = line.split("=", 1)
        if key not in keys or key in values or not value:
            raise ValueError(f"{description} fields are invalid")
        values[key] = value
    if set(values) != keys:
        raise ValueError(f"{description} fields are incomplete")
    if re.fullmatch(r"[0-9a-f]{40}", values[f"{prefix}_SOURCE_COMMIT"]) is None:
        raise ValueError(f"{description} source commit is malformed")
    for suffix in ("IMAGE_SHA256", "ELF_SHA256", "APP_ELF_SHA256"):
        if re.fullmatch(r"[0-9a-f]{64}", values[f"{prefix}_{suffix}"]) is None:
            raise ValueError(f"{prefix}_{suffix} is malformed")
    if values[f"{prefix}_ELF_SHA256"] != values[f"{prefix}_APP_ELF_SHA256"]:
        raise ValueError(f"{description} ELF and embedded digest differ")
    if re.fullmatch(r"[1-9][0-9]*", values[f"{prefix}_IMAGE_SIZE"]) is None:
        raise ValueError(f"{description} image size is malformed")
    if (
        re.fullmatch(r"build-[a-z0-9-]+", values[f"{prefix}_BUILD_DIRECTORY"])
        is None
    ):
        raise ValueError(f"{description} build directory is malformed")
    return values


def load_lkg_lock(path: Path) -> dict[str, str]:
    return parse_lock(path, "LKG", LKG_LOCK_KEYS, "last-known-good lock")


def load_snapshot_lock(path: Path) -> dict[str, str]:
    return parse_lock(
        path,
        "SNAPSHOT",
        SNAPSHOT_LOCK_KEYS,
        "BoBe snapshot lock",
    )


def create_snapshot_lock(
    source_commit: str,
    build: Path,
    output: Path,
    profile: str,
) -> dict[str, str]:
    if re.fullmatch(r"[0-9a-f]{40}", source_commit) is None:
        raise ValueError("snapshot source commit is malformed")
    if profile not in PROFILES:
        raise ValueError(f"unsupported firmware profile {profile!r}")
    if build.is_symlink() or not build.is_dir():
        raise ValueError("snapshot build directory is unavailable or unsafe")
    if build.name != SNAPSHOT_BUILD_DIRECTORY:
        raise ValueError("snapshot build directory has an unexpected name")
    if output.exists() or output.is_symlink():
        raise ValueError("snapshot lock output already exists")
    image = build / "bobe_wb12.bin"
    elf = build / "bobe_wb12.elf"
    application = inspect_application(image)
    elf_sha256 = sha256_file(elf)
    if application["app_elf_sha256"] != elf_sha256:
        raise ValueError("snapshot ELF does not match the image descriptor")
    values = {
        "SNAPSHOT_SOURCE_COMMIT": source_commit,
        "SNAPSHOT_IMAGE_SHA256": str(application["image_sha256"]),
        "SNAPSHOT_ELF_SHA256": elf_sha256,
        "SNAPSHOT_APP_ELF_SHA256": str(application["app_elf_sha256"]),
        "SNAPSHOT_IMAGE_SIZE": str(application["image_size"]),
        "SNAPSHOT_BUILD_DIRECTORY": build.name,
    }
    manifest = {
        **application,
        "profile": profile,
        "elf_file_sha256": elf_sha256,
        "source_commit": source_commit,
        "source_dirty": False,
        "created_at": datetime.now(UTC).isoformat(timespec="seconds"),
    }
    write_json(Path(f"{image}.manifest.json"), manifest)
    descriptor = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
    with os.fdopen(descriptor, "w", encoding="ascii") as lock_file:
        for key in (
            "SNAPSHOT_SOURCE_COMMIT",
            "SNAPSHOT_IMAGE_SHA256",
            "SNAPSHOT_ELF_SHA256",
            "SNAPSHOT_APP_ELF_SHA256",
            "SNAPSHOT_IMAGE_SIZE",
            "SNAPSHOT_BUILD_DIRECTORY",
        ):
            lock_file.write(f"{key}={values[key]}\n")
    return values


def main() -> None:
    parser = argparse.ArgumentParser(
        prog="bobe-locks",
        description="Create and inspect BoBe firmware provenance locks.",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    create = subparsers.add_parser(
        "create-snapshot-lock",
        help="Record BoBe snapshot build digests into a new lock file.",
    )
    create.add_argument("--source-commit", required=True)
    create.add_argument("--build", type=Path, required=True)
    create.add_argument("--output", type=Path, required=True)
    create.add_argument("--profile", choices=sorted(PROFILES), required=True)
    show = subparsers.add_parser(
        "show",
        help="Parse and print an existing snapshot or last-known-good lock.",
    )
    show.add_argument("path", type=Path)
    show.add_argument(
        "--kind",
        choices=("snapshot", "lkg"),
        default="snapshot",
        help="Lock flavor to parse (default: snapshot).",
    )
    args = parser.parse_args()
    try:
        if args.command == "create-snapshot-lock":
            result = create_snapshot_lock(
                args.source_commit,
                args.build,
                args.output,
                args.profile,
            )
        elif args.kind == "snapshot":
            result = load_snapshot_lock(args.path)
        else:
            result = load_lkg_lock(args.path)
        json.dump(result, sys.stdout, indent=2, sort_keys=True)
        sys.stdout.write("\n")
    except (OSError, RuntimeError, ValueError) as error:
        parser.error(str(error))


if __name__ == "__main__":
    main()
