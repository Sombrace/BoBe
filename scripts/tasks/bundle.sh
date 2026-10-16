#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
source "$ROOT/scripts/lib/locales.sh"
version="${1:-0.0.0}"
APP="build/BoBe.app"
XCODE_APP="BoBeMacUI/.build/xcode-derived/Build/Products/Release/BoBe.app"
test -d "$XCODE_APP" || {
    echo "Xcode release app not found: $XCODE_APP" >&2
    exit 1
}
rm -rf "$APP"
ditto "$XCODE_APP" "$APP"

./scripts/check-agent-runtime.sh
ditto build/agent-runtime/node "$APP/Contents/MacOS/node"
ditto build/agent-runtime/agent "$APP/Contents/Resources/agent"

ditto BoBeService/target/release/bobe "$APP/Contents/MacOS/bobe-daemon"
./scripts/tasks/notices.sh "$APP/Contents/Resources/licenses" \
    BoBeMacUI/.build/xcode-derived/SourcePackages/checkouts
./scripts/check-agent-runtime.sh "$APP/Contents/MacOS" "$APP/Contents/Resources/agent"

/usr/libexec/PlistBuddy -c "Set :CFBundleVersion ${version}" "$APP/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString ${version}" "$APP/Contents/Info.plist"

strip -x "$APP/Contents/MacOS/bobe-daemon" 2>/dev/null || true
strip -x "$APP/Contents/MacOS/BoBe" 2>/dev/null || true

while IFS= read -r dependency; do
    framework=${dependency#@rpath/}
    framework=${framework%%/Versions/*}
    test -d "$APP/Contents/Frameworks/$framework" || {
        echo "Missing embedded framework: $framework" >&2
        exit 1
    }
done < <(otool -L "$APP/Contents/MacOS/BoBe" | grep -o '@rpath/[^ ]*\.framework/[^ ]*' || true)
for resource_bundle in textual_Textual.bundle swiftui-math_SwiftUIMath.bundle; do
    test -d "$APP/Contents/Resources/$resource_bundle" || {
        echo "Missing SwiftPM resource bundle: $resource_bundle" >&2
        exit 1
    }
done
bobe_check_bundle_locales "$APP/Contents/Resources" UI.strings
test -f "$APP/Contents/Resources/PrivacyInfo.xcprivacy" || {
    echo "Missing BoBe privacy manifest" >&2
    exit 1
}
./scripts/check-privacy-manifest.sh \
    "$APP/Contents/Resources/PrivacyInfo.xcprivacy"
./scripts/check-macos-deployment-target.sh \
    --require-mach-o "$APP/Contents/MacOS/BoBe" \
    --require-mach-o "$APP/Contents/MacOS/bobe-daemon"

echo "Bundle created at $APP"
echo "  Backend: $(wc -c < "$APP/Contents/MacOS/bobe-daemon" | tr -d ' ') bytes"
echo "  Agent runtime: $(du -sh "$APP/Contents/Resources/agent" | awk '{print $1}')"
echo "  Node: $(wc -c < "$APP/Contents/MacOS/node" | tr -d ' ') bytes"
echo "  Frontend: $(wc -c < "$APP/Contents/MacOS/BoBe" | tr -d ' ') bytes"
