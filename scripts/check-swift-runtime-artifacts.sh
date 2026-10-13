#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP="${1:-$ROOT/BoBeMacUI/.build/xcode-derived/Build/Products/Debug/BoBe.app}"
EXECUTABLE="$APP/Contents/MacOS/BoBe"

[[ -x "$EXECUTABLE" ]]
[[ -f "$APP/Contents/Frameworks/Sparkle.framework/Versions/B/Sparkle" ]]
[[ -d "$APP/Contents/Resources/textual_Textual.bundle" ]]
[[ -d "$APP/Contents/Resources/swiftui-math_SwiftUIMath.bundle" ]]
[[ -f "$APP/Contents/Resources/en.lproj/UI.strings" ]]
[[ -f "$APP/Contents/Resources/PrivacyInfo.xcprivacy" ]]

LINK_IMAGE="$EXECUTABLE"
if [[ -f "$APP/Contents/MacOS/BoBe.debug.dylib" ]]; then
    LINK_IMAGE="$APP/Contents/MacOS/BoBe.debug.dylib"
fi

otool -L "$LINK_IMAGE" | grep -q '@rpath/Sparkle.framework/Versions/B/Sparkle'
otool -l "$EXECUTABLE" | grep -q '@executable_path/../Frameworks'

echo "Swift runtime artifacts ok ($APP)"
