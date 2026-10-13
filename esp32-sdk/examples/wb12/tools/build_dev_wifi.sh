#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="${1:-$ROOT/private/dev-wifi.env}"
BUILD="$ROOT/build-dev-wifi"
SDKCONFIG="$ROOT/build-dev-wifi.sdkconfig"
DEFAULTS="$ROOT/private/.dev-wifi.sdkconfig.defaults"

if [[ $# -gt 1 || ! -f "$ENV_FILE" || -L "$ENV_FILE" ]]; then
    printf 'Usage: %s [private/dev-wifi.env]\n' "$0" >&2
    exit 2
fi

set -a
# shellcheck source=/dev/null
source "$ENV_FILE"
set +a

: "${BOBE_WIFI_SSID:?BOBE_WIFI_SSID is required}"
: "${BOBE_WIFI_PASSWORD:?BOBE_WIFI_PASSWORD is required}"

if [[ "$BOBE_WIFI_SSID" == *$'\n'* ||
      "$BOBE_WIFI_PASSWORD" == *$'\n'* ]]; then
    printf 'Wi-Fi values must each fit on one line.\n' >&2
    exit 1
fi

kconfig_escape() {
    local value="$1"
    value="${value//\\/\\\\}"
    value="${value//\"/\\\"}"
    printf '%s' "$value"
}

cleanup() {
    rm -f -- "$DEFAULTS"
}
trap cleanup EXIT

rm -rf -- "$BUILD"
rm -f -- "$SDKCONFIG"
umask 077
{
    printf 'CONFIG_BOBE_ENABLE_DEV_WIFI_BOOTSTRAP=y\n'
    printf 'CONFIG_BOBE_WIFI_SSID="%s"\n' \
        "$(kconfig_escape "$BOBE_WIFI_SSID")"
    printf 'CONFIG_BOBE_WIFI_PASSWORD="%s"\n' \
        "$(kconfig_escape "$BOBE_WIFI_PASSWORD")"
} >"$DEFAULTS"

"$ROOT/tools/idf.sh" \
    -B "$BUILD" \
    -D "SDKCONFIG=$SDKCONFIG" \
    -D "SDKCONFIG_DEFAULTS=$ROOT/sdkconfig.defaults;$ROOT/sdkconfig.provisioning.defaults;$ROOT/sdkconfig.edge.defaults;$DEFAULTS" \
    build

grep -Fxq 'CONFIG_BOBE_ENABLE_DEV_WIFI_BOOTSTRAP=y' "$SDKCONFIG"
uv run --group hil --locked python -m bobe_hil.image create-manifest \
    "$BUILD/bobe_wb12.bin" \
    "$BUILD/bobe_wb12.elf" \
    --profile edge \
    --output "$BUILD/bobe_wb12.bin.manifest.json"

printf 'Development Wi-Fi image: %s\n' "$BUILD/bobe_wb12.bin"
printf 'This ignored build embeds the credentials from %s.\n' "$ENV_FILE"
