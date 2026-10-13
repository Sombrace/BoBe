#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="$ROOT/hil/fixture/pico2"
BUILD="$ROOT/build-pico-fixture"
SDK="$ROOT/.deps/pico-sdk"
TOOLCHAIN="$ROOT/.deps/pico-riscv-toolchain"

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
