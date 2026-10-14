#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TASK_DIR="$ROOT/scripts/tasks"
cd "$ROOT"
version="${1:-$(python3 -c 'import pathlib, tomllib; print(tomllib.loads(pathlib.Path("BoBeService/Cargo.toml").read_text())["package"]["version"])')}"
"$TASK_DIR/build-backend.sh" release
compatibility="$(BoBeService/target/release/bobe version --json)"
runtime_version="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["version"])' <<<"$compatibility")"
if [[ "$runtime_version" != "$version" ]]; then
    echo "Standalone version $version does not match compiled daemon version $runtime_version" >&2
    exit 1
fi
platform="$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)"
root="build/bobe-daemon-$version-$platform"
archive="$root.tar.gz"
rm -rf "$root" "$archive"
mkdir -p "$root/bin"
install -m 0755 BoBeService/target/release/bobe "$root/bin/bobe"
./scripts/tasks/notices.sh "$root/bin/licenses"
./scripts/check-agent-runtime.sh
install -m 0755 build/agent-runtime/node "$root/bin/node"
cp -RP build/agent-runtime/agent "$root/bin/agent"
install -m 0755 scripts/install-standalone.sh "$root/install.sh"
install -m 0755 scripts/uninstall-standalone.sh "$root/uninstall.sh"
if [[ "$(uname -s)" == Darwin ]]; then
    ./scripts/check-macos-deployment-target.sh --require-mach-o "$root/bin/bobe"
fi
python3 - "$compatibility" "$platform" >"$root/compatibility.json" <<'PY'
import json
import sys

compatibility = json.loads(sys.argv[1])
compatibility["platform"] = sys.argv[2]
print(json.dumps(compatibility, separators=(",", ":")))
PY
COPYFILE_DISABLE=1 tar -czf "$archive" -C build "$(basename "$root")"
if command -v shasum >/dev/null 2>&1; then
    checksum() { shasum -a 256 "$1" | awk '{print $1}'; }
else
    checksum() { sha256sum "$1" | awk '{print $1}'; }
fi
archive_name="$(basename "$archive")"
printf '%s  %s\n' "$(checksum "$archive")" "$archive_name" >"$archive.sha256"
echo "$archive"
