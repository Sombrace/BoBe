#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
source "$ROOT/scripts/lib/agent-runtime.sh"
identity="${1:-Developer ID Application}"
APP="build/BoBe.app"
./scripts/check-agent-runtime.sh "$APP/Contents/MacOS" "$APP/Contents/Resources/agent"
signing_dir="$ROOT/build/signing-$$"
mkdir -m 0700 "$signing_dir"
APP_ENTITLEMENTS="$signing_dir/app-entitlements.plist"
trap 'rm -rf "$signing_dir"' EXIT
[[ -f "$APP/Contents/embedded.provisionprofile" ]] || {
    echo "Provisioned Xcode app lacks embedded.provisionprofile" >&2
    exit 1
}
codesign -d --entitlements :- "$APP" >"$APP_ENTITLEMENTS" 2>/dev/null
grep -Fq 'com.bobe.app.endpoint-credential' "$APP_ENTITLEMENTS" || {
    echo "Provisioned Xcode app lacks expanded endpoint credential entitlements" >&2
    exit 1
}
SPARKLE="$APP/Contents/Frameworks/Sparkle.framework"
SPARKLE_VERSION="$SPARKLE/Versions/B"
SIGN_ARGS=(--force --sign "$identity")
if [[ "$identity" != "-" ]]; then
    SIGN_ARGS+=(--options runtime --timestamp)
fi

bobe_sign_agent_modules "$APP/Contents/Resources/agent" "${SIGN_ARGS[@]}"
codesign "${SIGN_ARGS[@]}" \
    --entitlements BoBeMacUI/node.entitlements.plist \
    "$APP/Contents/MacOS/node"

codesign "${SIGN_ARGS[@]}" \
    "$APP/Contents/MacOS/bobe-daemon"

if [[ -d "$SPARKLE" ]]; then
    codesign "${SIGN_ARGS[@]}" \
        "$SPARKLE_VERSION/XPCServices/Installer.xpc"
    if [[ -d "$SPARKLE_VERSION/XPCServices/Downloader.xpc" ]]; then
        codesign "${SIGN_ARGS[@]}" \
            --preserve-metadata=entitlements \
            "$SPARKLE_VERSION/XPCServices/Downloader.xpc"
    fi
    codesign "${SIGN_ARGS[@]}" \
        "$SPARKLE_VERSION/Autoupdate"
    codesign "${SIGN_ARGS[@]}" \
        "$SPARKLE_VERSION/Updater.app"
    codesign "${SIGN_ARGS[@]}" \
        "$SPARKLE"
fi

for framework in "$APP/Contents/Frameworks"/*.framework; do
    [[ -d "$framework" && "$framework" != "$SPARKLE" ]] || continue
    codesign "${SIGN_ARGS[@]}" "$framework"
done

codesign "${SIGN_ARGS[@]}" \
    --entitlements "$APP_ENTITLEMENTS" \
    "$APP"

codesign --verify --deep --strict --verbose=2 "$APP"
if [[ "$identity" != "-" ]]; then
    ./scripts/check-macos-keychain-signing.sh "$APP"
fi
echo "Signing verified successfully"
