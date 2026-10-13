#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TASK_DIR="$ROOT/scripts/tasks"
APP="$ROOT/BoBeMacUI/.build/xcode-derived/Build/Products/Debug/BoBe.app"

cd "$ROOT"
if [[ -n "${BOBE_DEVELOPMENT_TEAM:-}" ]]; then
    signing=(
        CODE_SIGN_STYLE=Automatic
        DEVELOPMENT_TEAM="$BOBE_DEVELOPMENT_TEAM"
        CODE_SIGN_IDENTITY="Apple Development"
    )
else
    signing=(CODE_SIGNING_ALLOWED=NO)
    echo "Unsigned build: no Keychain, so the app gets a fresh credential from its own daemon at each launch and forgets pairings to other daemons. Set BOBE_DEVELOPMENT_TEAM to sign."
fi
"$TASK_DIR/build-backend.sh" debug
BOBE_SKIP_TOOL_CHECK=1 "$TASK_DIR/xcode.sh"
cargo build --manifest-path PairingFFI/Cargo.toml --release --locked
xcodebuild \
    -project BoBeMacUI/BoBe.xcodeproj \
    -scheme BoBe \
    -configuration Debug \
    -destination 'platform=macOS,arch=arm64' \
    -derivedDataPath BoBeMacUI/.build/xcode-derived \
    -onlyUsePackageVersionsFromResolvedFile \
    "${signing[@]}" \
    build -quiet
cp BoBeService/target/debug/bobe "$APP/Contents/MacOS/bobe-daemon"
echo "Launching BoBe..."
BOBE_NODE_PATH="$ROOT/build/agent-runtime/node" \
    BOBE_AGENT_ENTRY="$ROOT/build/agent-runtime/agent/dist/main.js" \
    "$APP/Contents/MacOS/BoBe"
