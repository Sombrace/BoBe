#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TASK_DIR="$ROOT/scripts/tasks"
cd "$ROOT"
if [[ "$(uname -s)" != Darwin ]]; then
    echo "Sparkle's sign_update runs only on macOS. On this host, build with scripts/tasks/standalone.sh, then sign the archive on the release Mac with scripts/tasks/standalone-sign-update.sh." >&2
    exit 2
fi

version="${1:-$(python3 -c 'import pathlib, tomllib; print(tomllib.loads(pathlib.Path("BoBeService/Cargo.toml").read_text())["package"]["version"])')}"
private_key_file="${2:-}"
platform="$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)"

"$TASK_DIR/standalone.sh" "$version"
"$TASK_DIR/standalone-sign-update.sh" "build/bobe-daemon-${version}-${platform}.tar.gz" "$private_key_file"
