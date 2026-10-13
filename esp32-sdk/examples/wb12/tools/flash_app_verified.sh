#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 2 ]]; then
    printf 'Usage: %s <serial-port> <application.bin>\n' \
        "$0" >&2
    exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PORT="$1"
IMAGE="$2"
MANIFEST="$IMAGE.manifest.json"

if [[ "$PORT" != /dev/cu.* && "$PORT" != /dev/tty.* ]]; then
    printf 'Serial port must be a /dev/cu.* or /dev/tty.* device.\n' >&2
    exit 1
fi
[[ -f "$IMAGE" && ! -L "$IMAGE" &&
   -f "$MANIFEST" && ! -L "$MANIFEST" ]] || {
    printf 'Application or manifest is missing or unsafe.\n' >&2
    exit 1
}

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cd "$ROOT"

uv run --locked python tools/bobe_image.py inspect-app \
    "$IMAGE" \
    --manifest "$MANIFEST" \
    --require-clean \
    --require-signature >"$TMP/image-info.json"
authorized_image="$TMP/authorized-app.bin"
cp "$IMAGE" "$authorized_image"
chmod 600 "$authorized_image"
expected_sha="$(
    uv run --locked python -c \
        'import json,sys; print(json.load(open(sys.argv[1]))["image_sha256"])' \
        "$TMP/image-info.json"
)"
authorized_sha="$(shasum -a 256 "$authorized_image" | awk '{print $1}')"
[[ "$authorized_sha" == "$expected_sha" ]] || {
    printf 'Application changed while creating the authorized flash copy.\n' >&2
    exit 1
}
image_size="$(stat -f '%z' "$authorized_image")"
(( image_size > 0 && image_size <= 0x500000 )) || {
    printf 'Authorized application exceeds the fixed 5 MiB ota_0 slot.\n' >&2
    exit 1
}

uv run --locked esptool \
    --chip esp32s3 --port "$PORT" --baud 230400 \
    --before usb-reset --after no-reset \
    read-mac >"$TMP/read-mac.txt"
device_mac="$(
    awk '/^MAC:/ { print $2; exit }' "$TMP/read-mac.txt" |
        tr '[:upper:]' '[:lower:]'
)"
[[ "$device_mac" =~ ^([0-9a-f]{2}:){5}[0-9a-f]{2}$ ]] || {
    printf 'Connected ESP did not return a valid MAC address.\n' >&2
    exit 1
}
printf 'Connected ESP32-S3 MAC: %s\n' "$device_mac"

uv run --locked esptool \
    --chip esp32s3 --port "$PORT" --baud 230400 \
    --before no-reset --after no-reset \
    get-security-info >"$TMP/security-info.txt"
grep -q '^Secure Boot: Disabled$' "$TMP/security-info.txt"
grep -q '^Flash Encryption: Disabled$' "$TMP/security-info.txt"

uv run --locked esptool \
    --chip esp32s3 --port "$PORT" --baud 230400 \
    --before no-reset --after no-reset \
    read-flash 0xf000 0x2000 "$TMP/otadata.bin"
uv run --locked python tools/bobe_image.py inspect-otadata \
    "$TMP/otadata.bin" \
    --require-slot 0 >"$TMP/otadata.json"

printf 'Writing %s-byte application with SHA-256 %s\n' \
    "$image_size" \
    "$authorized_sha"
uv run --locked esptool \
    --chip esp32s3 --port "$PORT" --baud 230400 \
    --before no-reset --after no-reset \
    write-flash \
    --flash-mode keep --flash-freq keep --flash-size keep \
    0x20000 "$authorized_image"
uv run --locked esptool \
    --chip esp32s3 --port "$PORT" --baud 230400 \
    --before no-reset --after hard-reset \
    verify-flash \
    --flash-mode keep --flash-freq keep --flash-size keep \
    0x20000 "$authorized_image"

printf 'Application flash and read-back verification completed.\n'
