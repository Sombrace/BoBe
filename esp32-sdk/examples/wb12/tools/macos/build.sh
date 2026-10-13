#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SOURCE="$ROOT/tools/macos/bobe-capture.swift"
BUILD_ROOT="$ROOT/.tmp/bobe-capture"
APP="$BUILD_ROOT/BoBeCapture.app"
CONTENTS="$APP/Contents"
BINARY="$CONTENTS/MacOS/bobe-capture"
PLIST="$CONTENTS/Info.plist"
BUNDLE_ID="com.johnkozaris.espBobeToy.bobe-capture"
BUILD_LOG="$BUILD_ROOT/build.log"
TARGET="$(uname -m)-apple-macos14.0"
QUIET=false

if [[ $# -gt 1 || ( $# -eq 1 && "$1" != "--quiet" ) ]]; then
    printf '{"error":{"code":"INVALID_ARGUMENT","message":"Usage: tools/macos/build.sh [--quiet]"},"ok":false}\n' >&2
    exit 2
fi
if [[ ${1:-} == "--quiet" ]]; then
    QUIET=true
fi

needs_build=false
if [[ ! -x "$BINARY" ||
      ! -f "$PLIST" ||
      "$SOURCE" -nt "$BINARY" ||
      "${BASH_SOURCE[0]}" -nt "$BINARY" ]] ||
   ! /usr/bin/codesign --verify --deep --strict "$APP" >/dev/null 2>&1; then
    needs_build=true
fi

if [[ "$needs_build" == true ]]; then
    mkdir -p "$CONTENTS/MacOS"
    cat >"$PLIST" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>bobe-capture</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>BoBe Capture</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>1.1.0</string>
    <key>CFBundleVersion</key>
    <string>2</string>
    <key>LSUIElement</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>14.0</string>
    <key>NSCameraUsageDescription</key>
    <string>Capture deterministic hardware-in-the-loop validation frames from a specifically selected camera.</string>
    <key>NSMicrophoneUsageDescription</key>
    <string>BoBe Capture does not record audio; this description is present only for an explicit bundled privacy declaration.</string>
</dict>
</plist>
EOF
    rm -f "$BINARY.new"
    if ! (
        xcrun swiftc \
            -swift-version 5 \
            -target "$TARGET" \
            -O \
            "$SOURCE" \
            -o "$BINARY.new" \
            -framework AVFoundation \
            -framework CoreGraphics \
            -framework CoreImage \
            -framework CoreML \
            -framework CryptoKit \
            -framework ImageIO \
            -framework Vision &&
        mv "$BINARY.new" "$BINARY" &&
        /usr/bin/codesign --force --sign - --identifier "$BUNDLE_ID" "$APP"
    ) >"$BUILD_LOG" 2>&1; then
        rm -f "$BINARY.new"
        printf '{"error":{"code":"BUILD_FAILED","details":{"log":".tmp/bobe-capture/build.log"},"message":"Swift compilation or app signing failed"},"ok":false}\n' >&2
        exit 70
    fi
    if [[ "$QUIET" == false && -s "$BUILD_LOG" ]]; then
        cat "$BUILD_LOG" >&2
    fi
    rm -f "$BUILD_LOG"
fi

if [[ "$QUIET" == false ]]; then
    "$BINARY" --version
fi
