#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXPECTED="26.0"
REQUIRED_MACH_O=()
REQUIRED_MACH_O_COUNT=0
APPCAST=""
APPCAST_VERSION=""

usage() {
    cat <<'EOF'
Usage: check-macos-deployment-target.sh [options]

Checks the canonical macOS deployment declarations. Existing generated Xcode
projects and known Mach-O artifacts are checked when present.

Options:
  --require-mach-o PATH  Require PATH to exist and declare minos 26.0.
  --appcast PATH         Require an appcast item for --version with minimum 26.0.
  --version VERSION      CFBundleVersion to check in --appcast.
EOF
}

while (($#)); do
    case "$1" in
        --require-mach-o)
            [[ $# -ge 2 ]] || { usage >&2; exit 2; }
            REQUIRED_MACH_O+=("$2")
            REQUIRED_MACH_O_COUNT=$((REQUIRED_MACH_O_COUNT + 1))
            shift 2
            ;;
        --appcast)
            [[ $# -ge 2 ]] || { usage >&2; exit 2; }
            APPCAST="$2"
            shift 2
            ;;
        --version)
            [[ $# -ge 2 ]] || { usage >&2; exit 2; }
            APPCAST_VERSION="$2"
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

cd "$ROOT"

python3 - "$EXPECTED" <<'PY'
from pathlib import Path
import plistlib
import re
import sys

expected = sys.argv[1]
failures: list[str] = []


def require(path: str, pattern: str, description: str) -> None:
    text = Path(path).read_text()
    if re.search(pattern, text, re.MULTILINE) is None:
        failures.append(f"{path}: expected {description}")


require("BoBeMacUI/Package.swift", r"\.macOS\(\"26\.0\"\)", "SwiftPM macOS 26 platform")
require(
    "BoBeMacUI/project.yml",
    r'^\s*macOS:\s*["\']26\.0["\']\s*$',
    "XcodeGen deploymentTarget macOS 26.0",
)
require(
    "BoBeMacUI/project.yml",
    r'^\s*MACOSX_DEPLOYMENT_TARGET:\s*["\']26\.0["\']\s*$',
    "Xcode MACOSX_DEPLOYMENT_TARGET 26.0",
)
require(
    "BoBeMacUI/project.yml",
    r'^\s*xcodeVersion:\s*["\']27\.0["\']\s*$',
    "XcodeGen Xcode 27 baseline",
)
require(
    "BoBeService/.cargo/config.toml",
    r'link-arg=-mmacosx-version-min=26\.0',
    "Darwin Rust linker minimum 26.0",
)
require(
    "scripts/tasks/build-backend.sh",
    r'export MACOSX_DEPLOYMENT_TARGET=26\.0',
    "Darwin native build minimum 26.0",
)

with Path("BoBeMacUI/BoBe/Resources/Info.plist").open("rb") as stream:
    plist = plistlib.load(stream)
if plist.get("LSMinimumSystemVersion") != expected:
    failures.append(
        "BoBeMacUI/BoBe/Resources/Info.plist: expected LSMinimumSystemVersion 26.0"
    )

project = Path("BoBeMacUI/BoBe.xcodeproj/project.pbxproj")
if project.exists():
    values = set(re.findall(r"MACOSX_DEPLOYMENT_TARGET = ([^;]+);", project.read_text()))
    if values != {expected}:
        failures.append(
            f"{project}: generated deployment targets are {sorted(values) or ['missing']}, expected only 26.0; run 'scripts/tasks/xcode.sh'"
        )

if failures:
    raise SystemExit("\n".join(failures))
PY

mach_o_minos() {
    local binary="$1"
    local output
    output="$(xcrun vtool -show-build "$binary" 2>/dev/null)" || {
        echo "$binary: could not inspect Mach-O build version" >&2
        return 1
    }
    local values
    values="$(awk '$1 == "minos" { print $2 }' <<<"$output" | sort -u)"
    if [[ -z "$values" ]]; then
        echo "$binary: no LC_BUILD_VERSION minos found" >&2
        return 1
    fi
    while IFS= read -r value; do
        [[ "$value" == "$EXPECTED" ]] || {
            echo "$binary: declares minos $value, expected $EXPECTED" >&2
            return 1
        }
    done <<<"$values"
}

if [[ "$(uname -s)" == Darwin ]]; then
    if ((REQUIRED_MACH_O_COUNT > 0)); then
        for binary in "${REQUIRED_MACH_O[@]}"; do
            [[ -f "$binary" ]] || {
                echo "Required Mach-O artifact not found: $binary" >&2
                exit 1
            }
            mach_o_minos "$binary"
        done
    fi
elif ((REQUIRED_MACH_O_COUNT > 0)); then
    echo "Mach-O artifacts can only be checked on macOS" >&2
    exit 1
fi

if [[ -n "$APPCAST" || -n "$APPCAST_VERSION" ]]; then
    [[ -n "$APPCAST" && -n "$APPCAST_VERSION" ]] || {
        echo "--appcast and --version must be supplied together" >&2
        exit 2
    }
    [[ -f "$APPCAST" ]] || {
        echo "Appcast not found: $APPCAST" >&2
        exit 1
    }
    python3 - "$APPCAST" "$APPCAST_VERSION" "$EXPECTED" <<'PY'
import sys
import xml.etree.ElementTree as ET

path, version, expected = sys.argv[1:]
root = ET.parse(path).getroot()
sparkle = "http://www.andymatuschak.org/xml-namespaces/sparkle"
matched = []
for item in root.findall("./channel/item"):
    enclosure = item.find("enclosure")
    item_version = item.findtext(f"{{{sparkle}}}version")
    if item_version is None and enclosure is not None:
        item_version = enclosure.get(f"{{{sparkle}}}version")
    if item_version != version:
        continue
    minimum = item.findtext(f"{{{sparkle}}}minimumSystemVersion")
    if minimum is None and enclosure is not None:
        minimum = enclosure.get(f"{{{sparkle}}}minimumSystemVersion")
    matched.append(minimum)

if not matched:
    raise SystemExit(f"{path}: no appcast item found for version {version}")
if any(value != expected for value in matched):
    raise SystemExit(
        f"{path}: version {version} minimum system values are {matched}, expected only {expected}"
    )
PY
fi

echo "macOS deployment floor verified at $EXPECTED"
