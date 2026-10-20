#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
python3 - "$ROOT" <<'PY'
import json
import re
import sys
from pathlib import Path

root = Path(sys.argv[1])
PACKAGE = "BoBeMacUI/Package.swift"
PROJECT = "BoBeMacUI/project.yml"
RESOLVED = "BoBeMacUI/Package.resolved"


def fail(message):
    raise SystemExit(f"Swift package pins: {message}")


def read(relative):
    path = root / relative
    if not path.is_file():
        fail(f"missing {relative}")
    return path.read_text(encoding="utf-8")


def source(url):
    normalized = url.strip().lower().rstrip("/")
    return normalized[:-4] if normalized.endswith(".git") else normalized


def identity(url):
    return source(url).rsplit("/", 1)[-1]


def add(pins, origin, url, version):
    name = identity(url)
    if name in pins:
        fail(f"{origin} declares {name} twice")
    pins[name] = (source(url), version)


swiftpm = {}
package = read(PACKAGE)
declarations = re.findall(r"\.package\(", package)
exact = re.findall(r'\.package\(\s*url:\s*"([^"]+)"\s*,\s*exact:\s*"([^"]+)"\s*\)', package)
if not exact or len(exact) != len(declarations):
    fail(f"every {PACKAGE} dependency must be declared as .package(url:exact:)")
for url, version in exact:
    add(swiftpm, PACKAGE, url, version)

block = re.findall(r"^packages:[ \t]*\n((?:(?:[ \t]+[^\n]*)?\n)*)", read(PROJECT), re.MULTILINE)
if len(block) != 1:
    fail(f"expected one top-level packages mapping in {PROJECT}, found {len(block)}")
xcodegen_entries = {}
current = None
for line in block[0].splitlines():
    if not line.strip():
        continue
    entry = re.fullmatch(r"  ([A-Za-z0-9_.-]+):[ \t]*", line)
    field = re.fullmatch(r"    ([A-Za-z]+):[ \t]*(\S+)[ \t]*", line)
    if entry is not None:
        current = entry.group(1)
        if current in xcodegen_entries:
            fail(f"{PROJECT} declares package {current} twice")
        xcodegen_entries[current] = {}
    elif field is not None and current is not None:
        xcodegen_entries[current][field.group(1)] = field.group(2).strip("\"'")
    else:
        fail(f"cannot parse {PROJECT} packages line: {line.strip()!r}")
xcodegen = {}
for name, fields in xcodegen_entries.items():
    if set(fields) != {"url", "exactVersion"}:
        fail(f"{PROJECT} package {name} must declare exactly url and exactVersion, found {sorted(fields)}")
    add(xcodegen, PROJECT, fields["url"], fields["exactVersion"])

try:
    resolved_document = json.loads(read(RESOLVED))
except json.JSONDecodeError as error:
    fail(f"{RESOLVED} is not valid JSON: {error}")
resolved = {}
for pin in resolved_document.get("pins", []):
    location = pin.get("location")
    version = pin.get("state", {}).get("version")
    if not isinstance(location, str) or not isinstance(version, str):
        fail(f"{RESOLVED} pin {pin.get('identity')!r} has no location and version")
    if pin.get("identity") != identity(location):
        fail(f"{RESOLVED} pin identity {pin.get('identity')!r} does not match {location}")
    add(resolved, RESOLVED, location, version)
if not resolved:
    fail(f"{RESOLVED} has no pins")

problems = []
if set(swiftpm) != set(xcodegen):
    problems.append(
        f"{PACKAGE} declares {sorted(swiftpm)} but {PROJECT} declares {sorted(xcodegen)}"
    )
for name in sorted(swiftpm):
    url, version = swiftpm[name]
    for origin, pins in ((PROJECT, xcodegen), (RESOLVED, resolved)):
        if name not in pins:
            if origin == RESOLVED:
                problems.append(f"{RESOLVED} has no pin for {name}")
            continue
        other_url, other_version = pins[name]
        if other_url != url:
            problems.append(f"{name} source is {url} in {PACKAGE} but {other_url} in {origin}")
        if other_version != version:
            problems.append(f"{name} is {version} in {PACKAGE} but {other_version} in {origin}")
if problems:
    print("Swift package pin drift:", *problems, sep="\n  ", file=sys.stderr)
    raise SystemExit(1)
print("Swift package pins match: " + ", ".join(f"{name} {swiftpm[name][1]}" for name in sorted(swiftpm)))
PY
