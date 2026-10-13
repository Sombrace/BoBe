#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TASK_DIR="$ROOT/scripts/tasks"
cd "$ROOT"
[[ $# -ge 1 ]] || { echo "Usage: $0 <version> <private-key-file>" >&2; exit 2; }
version="${1}"
private_key_file="${2:-}"
SPARKLE_BIN="BoBeMacUI/.build/artifacts/sparkle/Sparkle/bin"
if [[ ! -x "$SPARKLE_BIN/sign_update" ]]; then
    echo "Sparkle tools not found. Run: cd BoBeMacUI && swift package resolve"
    exit 1
fi
archive="build/BoBe-$version.zip"
if [[ ! -f "$archive" ]]; then
    echo "Sparkle archive not found: $archive. Run: $TASK_DIR/sparkle-zip.sh $version"
    exit 1
fi
ARGS=()
if [[ -n "$private_key_file" ]]; then
    if [[ "$private_key_file" != "-" && ! -f "$private_key_file" ]]; then
        echo "Sparkle private key file not found: $private_key_file"
        exit 1
    fi
    ARGS+=(-f "$private_key_file")
fi
"$SPARKLE_BIN/sign_update" "${ARGS[@]}" "$archive"
