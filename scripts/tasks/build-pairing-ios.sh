#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

if [[ "$(uname -s)" != Darwin ]]; then
    echo "The iOS pairing library needs macOS with Xcode" >&2
    exit 1
fi

targets=(aarch64-apple-ios aarch64-apple-ios-sim)
output="$ROOT/build/pairing-ios"
framework="$output/BoBePairingFFI.xcframework"
headers="$output/include"

rustup target add "${targets[@]}"
export IPHONEOS_DEPLOYMENT_TARGET=26.0
libraries=()
for target in "${targets[@]}"; do
    cargo build --manifest-path PairingFFI/Cargo.toml --release --locked --target "$target"
    libraries+=(-library "PairingFFI/target/$target/release/libbobe_pairing_ffi.a" -headers "$headers")
done

rm -rf "$framework" "$headers"
mkdir -p "$headers"
cp PairingFFI/include/bobe_pairing.h "$headers/"
cat > "$headers/module.modulemap" <<'MODULEMAP'
module BoBePairingFFI {
    header "bobe_pairing.h"
    export *
}
MODULEMAP
xcodebuild -create-xcframework "${libraries[@]}" -output "$framework"
rm -rf "$headers"
echo "Built $framework"
