#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCK="$ROOT/hil/fixture/pico2/toolchain.lock"
DEPS="$ROOT/.deps"
SDK="$DEPS/pico-sdk"
TOOLCHAIN="$DEPS/pico-riscv-toolchain"
DOWNLOADS="$DEPS/downloads"

# shellcheck disable=SC1090
source "$LOCK"

mkdir -p "$DEPS" "$DOWNLOADS"

if [[ ! -d "$SDK/.git" ]]; then
    git clone \
        --branch "$PICO_SDK_VERSION" \
        --depth 1 \
        --recurse-submodules \
        --shallow-submodules \
        https://github.com/raspberrypi/pico-sdk.git \
        "$SDK"
fi
actual_commit="$(git -C "$SDK" rev-parse HEAD)"
[[ "$actual_commit" == "$PICO_SDK_COMMIT" ]] || {
    printf 'Unexpected Pico SDK commit: %s\n' "$actual_commit" >&2
    exit 1
}

archive="$DOWNLOADS/riscv-toolchain-15-mac.zip"
if [[ ! -f "$archive" ]]; then
    curl --fail --location --output "$archive" "$PICO_RISCV_TOOLCHAIN_URL"
fi
actual_sha="$(shasum -a 256 "$archive" | awk '{print $1}')"
[[ "$actual_sha" == "$PICO_RISCV_TOOLCHAIN_SHA256" ]] || {
    printf 'Unexpected Pico RISC-V toolchain digest: %s\n' "$actual_sha" >&2
    exit 1
}

if [[ ! -d "$TOOLCHAIN" ]]; then
    temporary="$(mktemp -d "$DEPS/pico-riscv.XXXXXX")"
    trap 'rm -rf "$temporary"' EXIT
    unzip -q "$archive" -d "$temporary"
    extracted_gcc="$(
        find "$temporary" -type f -name 'riscv32-unknown-elf-gcc' -print -quit
    )"
    [[ -n "$extracted_gcc" ]] || {
        printf 'Official Pico toolchain archive contains no RISC-V compiler.\n' >&2
        exit 1
    }
    extracted_root="$(dirname "$(dirname "$extracted_gcc")")"
    mv "$extracted_root" "$TOOLCHAIN"
fi

"$TOOLCHAIN/bin/riscv32-unknown-elf-gcc" --version | head -1
printf 'Pico SDK %s (%s) ready.\n' "$PICO_SDK_VERSION" "$actual_commit"
