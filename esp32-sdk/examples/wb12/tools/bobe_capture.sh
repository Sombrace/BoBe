#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD="$ROOT/tools/macos/build.sh"
BINARY="$ROOT/.tmp/bobe-capture/BoBeCapture.app/Contents/MacOS/bobe-capture"

if [[ $# -eq 1 && "$1" == "--build" ]]; then
    exec "$BUILD"
fi

"$BUILD" --quiet
exec "$BINARY" "$@"
