#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

[[ $# -ge 1 ]] || { echo "Usage: $0 <version>" >&2; exit 2; }
version="$1"
ditto -c -k --sequesterRsrc --keepParent build/BoBe.app "build/BoBe-${version}.zip"
echo "Sparkle archive created: build/BoBe-${version}.zip"
