#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

usage() {
    echo "Usage: $0 [VERSION] [--daemon PATH]" >&2
    exit 2
}

version=""
daemon=""
while (($#)); do
    case "$1" in
        --daemon)
            [[ $# -ge 2 ]] || usage
            daemon="$2"
            shift 2
            ;;
        -*)
            usage
            ;;
        *)
            [[ -z "$version" ]] || usage
            version="$1"
            shift
            ;;
    esac
done

report=""
if [[ -n "$daemon" ]]; then
    [[ -f "$daemon" && -x "$daemon" ]] || {
        echo "Compiled daemon not found: $daemon" >&2
        exit 1
    }
    report="$("$daemon" version --json)"
fi

python3 - "$ROOT" "$version" "$daemon" "$report" <<'PY'
import json
import plistlib
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
requested, daemon, report = sys.argv[2:5]


def fail(message):
    raise SystemExit(f"release version check: {message}")


def read(relative):
    path = root / relative
    if not path.is_file():
        fail(f"missing {relative}")
    return path.read_text(encoding="utf-8")


def single(label, values):
    if len(values) != 1:
        fail(f"expected one {label}, found {len(values)}")
    return values[0]


def package_version(relative):
    table = single(
        f"[package] table in {relative}",
        re.findall(r"^\[package\][ \t]*\n(.*?)(?=^\[|\Z)", read(relative), re.MULTILINE | re.DOTALL),
    )
    return single(
        f"[package] version in {relative}",
        re.findall(r'^version[ \t]*=[ \t]*"([^"]*)"[ \t]*$', table, re.MULTILINE),
    )


def pairing_core_requirement(relative):
    dependency = single(
        f"bobe-pairing-core dependency in {relative}",
        re.findall(r"^bobe-pairing-core[ \t]*=[ \t]*\{([^}\n]*)\}[ \t]*$", read(relative), re.MULTILINE),
    )
    return single(
        f"bobe-pairing-core version requirement in {relative}",
        re.findall(r'(?:^|,)[ \t]*version[ \t]*=[ \t]*"([^"]*)"', dependency),
    )


def locked_version(relative, package):
    return single(
        f"{package} in {relative}",
        re.findall(
            rf'^\[\[package\]\]\nname = "{re.escape(package)}"\nversion = "([^"]*)"$',
            read(relative),
            re.MULTILINE,
        ),
    )


def json_document(relative):
    try:
        return json.loads(read(relative))
    except json.JSONDecodeError as error:
        fail(f"{relative} is not valid JSON: {error}")


def plist_string(relative, key):
    read(relative)
    with (root / relative).open("rb") as stream:
        value = plistlib.load(stream).get(key)
    if not isinstance(value, str):
        fail(f"{relative} has no string {key}")
    return value


version = requested or package_version("BoBeService/Cargo.toml")
if re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version) is None:
    fail(f"version must be MAJOR.MINOR.PATCH: {version!r}")

agent_package = json_document("BoBeAgent/package.json")
agent_lock = json_document("BoBeAgent/package-lock.json")
declarations = [
    ("BoBeService/Cargo.toml version", package_version("BoBeService/Cargo.toml"), version),
    ("PairingCore/Cargo.toml version", package_version("PairingCore/Cargo.toml"), version),
    ("PairingFFI/Cargo.toml version", package_version("PairingFFI/Cargo.toml"), version),
    (
        "BoBeService/Cargo.toml bobe-pairing-core requirement",
        pairing_core_requirement("BoBeService/Cargo.toml"),
        f"={version}",
    ),
    (
        "PairingFFI/Cargo.toml bobe-pairing-core requirement",
        pairing_core_requirement("PairingFFI/Cargo.toml"),
        f"={version}",
    ),
    ("BoBeService/Cargo.lock bobe", locked_version("BoBeService/Cargo.lock", "bobe"), version),
    (
        "BoBeService/Cargo.lock bobe-pairing-core",
        locked_version("BoBeService/Cargo.lock", "bobe-pairing-core"),
        version,
    ),
    (
        "PairingCore/Cargo.lock bobe-pairing-core",
        locked_version("PairingCore/Cargo.lock", "bobe-pairing-core"),
        version,
    ),
    (
        "PairingFFI/Cargo.lock bobe-pairing-core",
        locked_version("PairingFFI/Cargo.lock", "bobe-pairing-core"),
        version,
    ),
    (
        "PairingFFI/Cargo.lock bobe-pairing-ffi",
        locked_version("PairingFFI/Cargo.lock", "bobe-pairing-ffi"),
        version,
    ),
    ("BoBeAgent/package.json version", agent_package.get("version"), version),
    ("BoBeAgent/package-lock.json version", agent_lock.get("version"), version),
    (
        "BoBeAgent/package-lock.json root package version",
        agent_lock.get("packages", {}).get("", {}).get("version"),
        version,
    ),
    (
        "BoBeMacUI/BoBe/Resources/Info.plist CFBundleShortVersionString",
        plist_string("BoBeMacUI/BoBe/Resources/Info.plist", "CFBundleShortVersionString"),
        version,
    ),
    (
        "BoBeMacUI/BoBe/Resources/Info.plist CFBundleVersion",
        plist_string("BoBeMacUI/BoBe/Resources/Info.plist", "CFBundleVersion"),
        version,
    ),
    (
        "BoBeiOS/BoBe/Resources/Info.plist CFBundleShortVersionString",
        plist_string("BoBeiOS/BoBe/Resources/Info.plist", "CFBundleShortVersionString"),
        version,
    ),
]
if daemon:
    try:
        compiled = json.loads(report).get("version")
    except (json.JSONDecodeError, AttributeError):
        compiled = None
    declarations.append((f"{daemon} version --json", compiled, version))

mismatches = [
    f"{label} is {actual!r}, expected {expected!r}"
    for label, actual, expected in declarations
    if actual != expected
]
if mismatches:
    print(f"Release version {version} does not match:", file=sys.stderr)
    for mismatch in mismatches:
        print(f"  {mismatch}", file=sys.stderr)
    raise SystemExit(1)
print(f"Release version {version} matches {len(declarations)} declarations")
PY
