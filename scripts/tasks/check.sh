#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TASK_DIR="$ROOT/scripts/tasks"
export CARGO_INCREMENTAL=0
export CARGO_BUILD_WARNINGS=deny

cd "$ROOT"
"$TASK_DIR/bootstrap-tools.sh" --check
"$TASK_DIR/build-agent.sh"
export BOBE_NODE_PATH="$ROOT/build/agent-runtime/node"
export BOBE_AGENT_ENTRY="$ROOT/build/agent-runtime/agent/dist/main.js"
./scripts/check-agent-runtime.sh

cargo fmt --manifest-path PairingCore/Cargo.toml --check
cargo test --manifest-path PairingCore/Cargo.toml --locked
cargo fmt --manifest-path PairingFFI/Cargo.toml --check
cargo test --manifest-path PairingFFI/Cargo.toml --locked
cargo build --manifest-path PairingFFI/Cargo.toml --release --locked

cargo fmt --manifest-path BoBeService/Cargo.toml --check
cargo clippy --manifest-path BoBeService/Cargo.toml --quiet --locked
cargo test --manifest-path BoBeService/Cargo.toml --quiet --locked
cargo build --manifest-path BoBeService/Cargo.toml --quiet --locked

for crate in BoBeService PairingCore PairingFFI; do
    (
        cd "$crate"
        ../.tools/bin/cargo-deny check --hide-inclusion-graph \
            advisories licenses bans sources
    )
    ./.tools/bin/cargo-machete --with-metadata "$crate"
done

./scripts/check-cross-language-constants.sh
./scripts/check-bodylink-contract.sh
./scripts/check-macos-deployment-target.sh
bash -n scripts/check-macos-keychain-signing.sh
./scripts/check-privacy-manifest.sh
./scripts/check-data-paths.sh
python3 scripts/check-standalone-update-manifest.py
./scripts/check-localization-catalogs.sh

(
    cd BoBeMacUI
    ../.tools/bin/swiftlint lint --quiet
)
./scripts/check-standalone-install.sh
BOBE_SKIP_TOOL_CHECK=1 "$TASK_DIR/xcode.sh"
xcodebuild \
    -project BoBeMacUI/BoBe.xcodeproj \
    -scheme BoBe \
    -configuration Debug \
    -destination 'platform=macOS,arch=arm64' \
    -derivedDataPath BoBeMacUI/.build/xcode-derived \
    -onlyUsePackageVersionsFromResolvedFile \
    -only-testing:BoBeTests \
    CODE_SIGNING_ALLOWED=NO \
    test
./scripts/check-macos-deployment-target.sh \
    --require-mach-o BoBeMacUI/.build/xcode-derived/Build/Products/Debug/BoBe.app/Contents/MacOS/BoBe
./scripts/check-swift-runtime-artifacts.sh \
    BoBeMacUI/.build/xcode-derived/Build/Products/Debug/BoBe.app

"$TASK_DIR/build-pairing-ios.sh"
(cd BoBeiOS && ../.tools/bin/xcodegen generate --quiet)
xcodebuild \
    -project BoBeiOS/BoBe.xcodeproj \
    -scheme BoBe \
    -configuration Debug \
    -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath BoBeiOS/.build/xcode-derived \
    CODE_SIGNING_ALLOWED=NO \
    build

if [[ -d BoBeService/target ]]; then
    target_kib="$(du -sk BoBeService/target | awk '{print $1}')"
    if (( target_kib > 15 * 1024 * 1024 )); then
        echo "Rust target is $((target_kib / 1024 / 1024)) GiB; cleaning the 15 GiB cache limit"
        cargo clean --manifest-path BoBeService/Cargo.toml
    fi
fi
