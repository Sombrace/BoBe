#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TASK_DIR="$ROOT/scripts/tasks"
cd "$ROOT"
"$TASK_DIR/xcode.sh"
cargo build --manifest-path PairingFFI/Cargo.toml --release --locked
: "${BOBE_DEVELOPMENT_TEAM:?Set BOBE_DEVELOPMENT_TEAM}"
: "${BOBE_PROVISIONING_PROFILE_SPECIFIER:?Set BOBE_PROVISIONING_PROFILE_SPECIFIER}"
cd BoBeMacUI
xcodebuild \
    -project BoBe.xcodeproj \
    -scheme BoBe \
    -configuration Release \
    -destination 'platform=macOS,arch=arm64' \
    -derivedDataPath .build/xcode-derived \
    -onlyUsePackageVersionsFromResolvedFile \
    ARCHS=arm64 \
    DEVELOPMENT_TEAM="$BOBE_DEVELOPMENT_TEAM" \
    BOBE_PROVISIONING_PROFILE_SPECIFIER="$BOBE_PROVISIONING_PROFILE_SPECIFIER" \
    CODE_SIGN_STYLE=Manual \
    CODE_SIGN_IDENTITY="${BOBE_CODE_SIGN_IDENTITY:-Developer ID Application}" \
    OTHER_CODE_SIGN_FLAGS="--timestamp --options runtime" \
    build -quiet
