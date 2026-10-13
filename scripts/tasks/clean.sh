#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

for manifest in BoBeService/Cargo.toml PairingCore/Cargo.toml PairingFFI/Cargo.toml; do
    cargo clean --manifest-path "$manifest"
done
(cd BoBeMacUI && swift package clean)
rm -rf BoBeMacUI/.build/xcode-derived BoBeMacUI/BoBe.xcodeproj build/
