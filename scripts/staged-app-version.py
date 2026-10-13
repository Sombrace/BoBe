#!/usr/bin/env python3

from __future__ import annotations

import pathlib
import plistlib
import sys
import zipfile


def fail(message: str) -> None:
    raise SystemExit(message)


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: staged-app-version.py ARCHIVES_DIR")

    archives = pathlib.Path(sys.argv[1])
    versions: set[str] = set()
    for archive in archives.glob("*.zip"):
        with zipfile.ZipFile(archive) as bundle:
            info_paths = [
                name
                for name in bundle.namelist()
                if name.endswith(".app/Contents/Info.plist")
            ]
            if not info_paths:
                continue
            if len(info_paths) != 1:
                fail(f"{archive}: expected one app Info.plist, found {len(info_paths)}")
            info = plistlib.loads(bundle.read(info_paths[0]))
            version = info.get("CFBundleVersion")
            if not isinstance(version, str) or not version:
                fail(f"{archive}: CFBundleVersion is missing")
            versions.add(version)

    if len(versions) != 1:
        fail(f"expected one staged app CFBundleVersion, found: {sorted(versions)}")
    print(versions.pop())


if __name__ == "__main__":
    main()
