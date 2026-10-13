#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

[[ $# -ge 1 ]] || { echo "Usage: $0 <version> [identity]" >&2; exit 2; }
version="$1"
identity="${2:-Developer ID Application}"
staging="build/dmg-staging"
dmg="build/BoBe-${version}.dmg"

rm -rf "$staging" "$dmg"
mkdir -p "$staging"
ditto build/BoBe.app "$staging/BoBe.app"
ln -s /Applications "$staging/Applications"
hdiutil create -volname "BoBe ${version}" \
    -srcfolder "$staging" \
    -ov -format UDZO \
    "$dmg"
rm -rf "$staging"
codesign -s "$identity" --timestamp -i com.bobe.app.dmg "$dmg"
echo "Signed DMG created: $dmg"
ls -lh "$dmg"
