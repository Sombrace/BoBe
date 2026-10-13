#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TOOLS_DIR="$ROOT/.tools"
BIN_DIR="$TOOLS_DIR/bin"
CACHE_DIR="$TOOLS_DIR/cache"
MODE="${1:-install}"

CARGO_DENY_VERSION="0.20.2"
CARGO_MACHETE_VERSION="0.9.2"
SWIFTLINT_VERSION="0.65.0"
XCODEGEN_VERSION="2.46.0"

tool_matches() {
    local binary="$1"
    local expected="$2"
    shift 2
    [[ -x "$binary" ]] && [[ "$("$binary" "$@" 2>/dev/null)" == "$expected" ]]
}

check_tools() {
    local failed=0
    tool_matches "$BIN_DIR/cargo-deny" "cargo-deny $CARGO_DENY_VERSION" --version || failed=1
    tool_matches "$BIN_DIR/cargo-machete" "$CARGO_MACHETE_VERSION" --version || failed=1
    tool_matches "$BIN_DIR/swiftlint" "$SWIFTLINT_VERSION" version || failed=1
    tool_matches "$BIN_DIR/xcodegen" "Version: $XCODEGEN_VERSION" --version || failed=1
    if [[ "$failed" -ne 0 ]]; then
        echo "Project developer tools are missing or have the wrong version." >&2
        echo "Run: scripts/tasks/bootstrap-tools.sh" >&2
        return 1
    fi
}

verified_archive() {
    local name="$1"
    local version="$2"
    local url="$3"
    local expected_sha="$4"
    local archive="$CACHE_DIR/$name-$version.zip"

    if [[ -f "$archive" ]]; then
        local current_sha
        current_sha="$(shasum -a 256 "$archive" | awk '{print $1}')"
        if [[ "$current_sha" != "$expected_sha" ]]; then
            rm -f "$archive"
        fi
    fi
    if [[ ! -f "$archive" ]]; then
        echo "Downloading $name $version..." >&2
        curl --fail --location --silent --show-error "$url" -o "$archive"
    fi

    local actual_sha
    actual_sha="$(shasum -a 256 "$archive" | awk '{print $1}')"
    if [[ "$actual_sha" != "$expected_sha" ]]; then
        echo "$name $version checksum mismatch" >&2
        rm -f "$archive"
        return 1
    fi
    printf '%s\n' "$archive"
}

install_single_binary_zip() {
    local name="$1"
    local version="$2"
    local url="$3"
    local sha="$4"
    local executable="$5"
    local archive
    archive="$(verified_archive "$name" "$version" "$url" "$sha")"
    local temp
    temp="$(mktemp -d "$TOOLS_DIR/$name.XXXXXX")"
    unzip -q "$archive" -d "$temp"
    install -m 755 "$temp/$executable" "$BIN_DIR/$executable"
    rm -rf "$temp"
}

if [[ "$MODE" == "--check" ]]; then
    check_tools
    exit 0
fi
if [[ "$MODE" != "install" ]]; then
    echo "Usage: $0 [--check]" >&2
    exit 2
fi

command -v cargo >/dev/null
command -v curl >/dev/null
command -v shasum >/dev/null
command -v unzip >/dev/null
mkdir -p "$BIN_DIR" "$CACHE_DIR"

if ! tool_matches "$BIN_DIR/cargo-deny" "cargo-deny $CARGO_DENY_VERSION" --version; then
    cargo install --locked --force --root "$TOOLS_DIR" \
        cargo-deny --version "$CARGO_DENY_VERSION"
fi
if ! tool_matches "$BIN_DIR/cargo-machete" "$CARGO_MACHETE_VERSION" --version; then
    cargo install --locked --force --root "$TOOLS_DIR" \
        cargo-machete --version "$CARGO_MACHETE_VERSION"
fi
if ! tool_matches "$BIN_DIR/swiftlint" "$SWIFTLINT_VERSION" version; then
    install_single_binary_zip \
        swiftlint "$SWIFTLINT_VERSION" \
        "https://github.com/realm/SwiftLint/releases/download/$SWIFTLINT_VERSION/portable_swiftlint.zip" \
        d6cb0aa7a2f5f1ef306fc9e37bcb54dc9a26facc8f7784ac0c3dd3eccf5c6ba6 \
        swiftlint
fi
if ! tool_matches "$BIN_DIR/xcodegen" "Version: $XCODEGEN_VERSION" --version; then
    archive="$(verified_archive \
        xcodegen "$XCODEGEN_VERSION" \
        "https://github.com/yonaskolb/XcodeGen/releases/download/$XCODEGEN_VERSION/xcodegen.zip" \
        4d9e34b62172d645eed6457cac13fc222569974098ef4ee9c3368bedf0196806)"
    temp="$(mktemp -d "$TOOLS_DIR/xcodegen.XXXXXX")"
    unzip -q "$archive" -d "$temp"
    rm -rf "$TOOLS_DIR/share/xcodegen"
    mkdir -p "$TOOLS_DIR/share"
    cp -R "$temp/xcodegen/share/xcodegen" "$TOOLS_DIR/share/xcodegen"
    install -m 755 "$temp/xcodegen/bin/xcodegen" "$BIN_DIR/xcodegen"
    rm -rf "$temp"
fi

check_tools
echo "Project developer tools are ready under $TOOLS_DIR"
