#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FINAL_BUILD="$ROOT/build-bobe-snapshot"
FINAL_LOCK="$FINAL_BUILD/bobe-snapshot.lock"
WORK=""
REPLACE=false
PROFILE="edge"
PUBLISHING=false
NEW_BUILD_PUBLISHED=false

usage() {
    printf 'Usage: %s [--profile fallback|ptt|edge] [--replace]\n' "$0"
    printf 'Builds normal BoBe WB-12 firmware from the committed Git snapshot\n'
    printf 'and records image/ELF provenance. It never flashes a device.\n'
}

while [[ $# -gt 0 ]]; do
    case "$1" in
    --replace)
        REPLACE=true
        shift
        ;;
    --profile)
        [[ $# -ge 2 ]] || {
            usage >&2
            exit 2
        }
        PROFILE="$2"
        shift 2
        ;;
    --help | -h)
        usage
        exit 0
        ;;
    *)
        usage >&2
        exit 2
        ;;
    esac
done

case "$PROFILE" in
fallback)
    PROFILE_DEFAULTS=()
    ;;
ptt)
    PROFILE_DEFAULTS=(sdkconfig.provisioning.defaults)
    ;;
edge)
    PROFILE_DEFAULTS=(
        sdkconfig.provisioning.defaults
        sdkconfig.edge.defaults
    )
    ;;
*)
    usage >&2
    exit 2
    ;;
esac

if [[ "$REPLACE" == true ]]; then
    [[ -d "$FINAL_BUILD" && ! -L "$FINAL_BUILD" ]] || {
        printf 'Snapshot replacement requires the prior build.\n' >&2
        exit 1
    }
else
    [[ ! -e "$FINAL_BUILD" && ! -L "$FINAL_BUILD" ]] || {
        printf 'Snapshot build already exists; use --replace.\n' >&2
        exit 1
    }
fi

SOURCE_COMMIT="$(git -C "$ROOT" rev-parse HEAD)"
SOURCE_SHORT="${SOURCE_COMMIT:0:12}"
SOURCE_DATE_EPOCH="$(git -C "$ROOT" show -s --format=%ct "$SOURCE_COMMIT")"
mkdir -p "$ROOT/.tmp"
WORK="$(mktemp -d "$ROOT/.tmp/bobe-snapshot-build.$SOURCE_SHORT.XXXXXX")"
cleanup() {
    local result=$?
    if [[ $result -ne 0 && "$PUBLISHING" == true ]]; then
        if [[ "$NEW_BUILD_PUBLISHED" == true &&
              -d "$FINAL_BUILD" && ! -L "$FINAL_BUILD" ]]; then
            chmod -R u+w "$FINAL_BUILD" 2>/dev/null || true
            rm -rf -- "$FINAL_BUILD"
        fi
        if [[ "$REPLACE" == true &&
              -d "$WORK/previous-build" ]]; then
            mv "$WORK/previous-build" "$FINAL_BUILD"
        fi
    fi
    if [[ -n "$WORK" &&
          "$WORK" == "$ROOT/.tmp/bobe-snapshot-build.$SOURCE_SHORT."* &&
          -d "$WORK" ]]; then
        chmod -R u+w "$WORK" 2>/dev/null || true
        rm -rf -- "$WORK"
    fi
    exit "$result"
}
trap cleanup EXIT

ARCHIVE_ROOT="$WORK/source"
SOURCE_ROOT="$ARCHIVE_ROOT/examples/wb12"
BUILD="$WORK/build-bobe-snapshot"
mkdir "$ARCHIVE_ROOT"
SDK_ROOT="$(cd "$ROOT/../.." && pwd)"
GIT_ROOT="$(git -C "$SDK_ROOT" rev-parse --show-toplevel)"
SDK_PREFIX="$(git -C "$SDK_ROOT" rev-parse --show-prefix)"
git -C "$GIT_ROOT" archive --format=tar "$SOURCE_COMMIT:${SDK_PREFIX%/}" |
    tar -xf - -C "$ARCHIVE_ROOT"
ln -s "$SDK_ROOT/.deps" "$ARCHIVE_ROOT/.deps"

DEFAULTS="$SOURCE_ROOT/sdkconfig.defaults"
for extra in ${PROFILE_DEFAULTS[@]+"${PROFILE_DEFAULTS[@]}"}; do
    DEFAULTS="$DEFAULTS;$SOURCE_ROOT/$extra"
done

export SOURCE_DATE_EPOCH
ESP_BOBE_SDK_PATH="$ARCHIVE_ROOT" "$SOURCE_ROOT/tools/idf.sh" \
    -B "$BUILD" \
    -D "SDKCONFIG=$BUILD/sdkconfig" \
    -D "SDKCONFIG_DEFAULTS=$DEFAULTS" \
    build

grep -Fxq 'CONFIG_IDF_TARGET="esp32s3"' "$BUILD/sdkconfig"
grep -Fxq 'CONFIG_APP_RETRIEVE_LEN_ELF_SHA=64' "$BUILD/sdkconfig"
if grep -Fxq 'CONFIG_BOBE_ENABLE_DEV_WIFI_BOOTSTRAP=y' "$BUILD/sdkconfig" ||
    grep -Eq '^CONFIG_BOBE_WIFI_(SSID|PASSWORD)="[^"]+"$' "$BUILD/sdkconfig"; then
    printf 'Snapshot sdkconfig enables development Wi-Fi or embeds Wi-Fi credentials.\n' >&2
    exit 1
fi
grep -a -q 'physical button event_id=' "$BUILD/bobe_wb12.elf"
IMAGE_SIZE="$(wc -c < "$BUILD/bobe_wb12.bin" | tr -d '[:space:]')"
(( IMAGE_SIZE <= 0x500000 )) || {
    printf 'Snapshot image exceeds the 5 MiB application slot.\n' >&2
    exit 1
}

(
    cd "$SOURCE_ROOT"
    UV_PROJECT_ENVIRONMENT="$WORK/lock-venv" \
        uv run --group hil --locked python \
        -m bobe_hil.locks create-snapshot-lock \
        --source-commit "$SOURCE_COMMIT" \
        --build "$BUILD" \
        --profile "$PROFILE" \
        --output "$BUILD/bobe-snapshot.lock"
)

PUBLISHING=true
if [[ "$REPLACE" == true ]]; then
    mv "$FINAL_BUILD" "$WORK/previous-build"
fi
mv "$BUILD" "$FINAL_BUILD"
NEW_BUILD_PUBLISHED=true
PUBLISHING=false
printf 'BoBe %s snapshot build created from %s.\n' "$PROFILE" "$SOURCE_COMMIT"
printf 'Image size: %s bytes\n' "$IMAGE_SIZE"
printf 'Lock: %s\n' "$FINAL_LOCK"
