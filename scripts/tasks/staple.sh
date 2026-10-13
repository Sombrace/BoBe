#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

[[ $# -ge 1 ]] || { echo "Usage: $0 <version>" >&2; exit 2; }
version="$1"
xcrun stapler staple build/BoBe.app
xcrun stapler validate build/BoBe.app
./scripts/check-macos-keychain-signing.sh build/BoBe.app
xcrun stapler staple "build/BoBe-${version}.dmg"
xcrun stapler validate "build/BoBe-${version}.dmg"
echo "App and DMG stapled and validated successfully"
