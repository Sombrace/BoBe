#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="$ROOT/hil/fixture/pico2"
BUILD="$ROOT/build-pico-fixture"

# shellcheck disable=SC1090
source "$ROOT/hil/fixture/pico2/toolchain.lock"
SDK="$ROOT/.deps/pico-sdk-$PICO_SDK_VERSION"
TOOLCHAIN="$ROOT/.deps/pico-riscv-toolchain-$PICO_SDK_TOOLS_VERSION"

"$ROOT/tools/bootstrap_pico_fixture.sh"

cp "$SDK/external/pico_sdk_import.cmake" "$SOURCE/pico_sdk_import.cmake"
trap 'rm -f "$SOURCE/pico_sdk_import.cmake"' EXIT

PICO_SDK_PATH="$SDK" \
PICO_TOOLCHAIN_PATH="$TOOLCHAIN" \
PICO_PLATFORM="rp2350-riscv" \
PICO_BOARD="pico2" \
cmake \
    -S "$SOURCE" \
    -B "$BUILD" \
    -DCMAKE_BUILD_TYPE=Release
cmake --build "$BUILD" --parallel

shasum -a 256 "$BUILD/bobe_fixture.uf2"
