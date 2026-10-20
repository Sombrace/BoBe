#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly GATES=(all core-policy ui ui-regenerate fixture capture)

usage() {
    local joined
    joined="$(IFS='|'; printf '%s' "${GATES[*]}")"
    printf 'Usage: %s [%s]\n' \
        "$0" "$joined"
    printf 'Compile/test evidence only; inspect this script for exact coverage.\n'
    printf 'No gate flashes or actuates hardware. ui-regenerate mutates references.\n'
}

ACTION="${1:-all}"
if [[ $# -gt 1 ]]; then
    usage >&2
    exit 2
fi
if [[ "$ACTION" == "--help" || "$ACTION" == "-h" ]]; then
    usage
    exit 0
fi
valid_action=false
for gate in "${GATES[@]}"; do
    if [[ "$ACTION" == "$gate" ]]; then
        valid_action=true
        break
    fi
done
[[ "$valid_action" == true ]] || {
    usage >&2
    exit 2
}

mkdir -p "$ROOT/.tmp"
CHECK_ROOT="$(mktemp -d "$ROOT/.tmp/check.XXXXXX")"

cleanup() {
    local status=$?
    if [[ $status -ne 0 && -d "$CHECK_ROOT" &&
          -n "$(find "$CHECK_ROOT" -mindepth 1 -maxdepth 1 -print -quit)" ]]; then
        local failure_root="$ROOT/.tmp/check-failures"
        local retained="$failure_root/$(basename "$CHECK_ROOT")"
        mkdir -p "$failure_root"
        chmod -R u+rwX "$CHECK_ROOT" 2>/dev/null || true
        mv "$CHECK_ROOT" "$retained"
        printf 'Failed gate artifacts retained at %s\n' "$retained" >&2
        return
    fi
    chmod -R u+w "$CHECK_ROOT" 2>/dev/null || true
    rm -rf "$CHECK_ROOT"
}
trap cleanup EXIT
export PYTHONPYCACHEPREFIX="$CHECK_ROOT/pycache"

require_config() {
    local sdkconfig="$1"
    local setting="$2"
    if ! grep -Fxq "$setting" "$sdkconfig"; then
        printf 'Expected %s in %s\n' "$setting" "$sdkconfig" >&2
        return 1
    fi
}

reject_config() {
    local sdkconfig="$1"
    local symbol="$2"
    if grep -Eq "^${symbol}=y$" "$sdkconfig"; then
        printf 'Unexpected %s in %s\n' "$symbol" "$sdkconfig" >&2
        return 1
    fi
}

reject_nonempty_string() {
    local sdkconfig="$1"
    local symbol="$2"
    if grep -Eq "^${symbol}=\".+\"$" "$sdkconfig"; then
        printf 'Unexpected non-empty %s in %s\n' "$symbol" "$sdkconfig" >&2
        return 1
    fi
}

build_firmware_profile() {
    local name="$1"
    local defaults="$2"
    local provisioning="$3"
    local esp_sr="$4"
    local provisioning_transport="${5:-softap}"
    local build="$CHECK_ROOT/firmware-$name"
    local sdkconfig="$CHECK_ROOT/sdkconfig-$name"
    local log="$CHECK_ROOT/$name-build.log"

    "$ROOT/tools/idf.sh" \
        -B "$build" \
        -D "SDKCONFIG=$sdkconfig" \
        -D "SDKCONFIG_DEFAULTS=$defaults" \
        build 2>&1 | tee "$log"

    require_config "$sdkconfig" 'CONFIG_IDF_TARGET="esp32s3"'
    require_config "$sdkconfig" 'CONFIG_ESP_TASK_WDT_PANIC=y'
    require_config "$sdkconfig" 'CONFIG_I2S_ISR_IRAM_SAFE=y'
    require_config "$sdkconfig" 'CONFIG_APP_RETRIEVE_LEN_ELF_SHA=64'
    require_config "$sdkconfig" 'CONFIG_PM_ENABLE=y'
    reject_config "$sdkconfig" CONFIG_FREERTOS_USE_TICKLESS_IDLE
    reject_config "$sdkconfig" CONFIG_PM_POWER_DOWN_CPU_IN_LIGHT_SLEEP
    reject_config "$sdkconfig" CONFIG_BOBE_ENABLE_DEV_WIFI_BOOTSTRAP
    reject_nonempty_string "$sdkconfig" CONFIG_BOBE_WIFI_SSID
    reject_nonempty_string "$sdkconfig" CONFIG_BOBE_WIFI_PASSWORD
    if [[ "$provisioning" == "yes" ]]; then
        require_config "$sdkconfig" 'CONFIG_BOBE_ENABLE_SECURE_PROVISIONING=y'
        if [[ "$provisioning_transport" == "ble" ]]; then
            require_config "$sdkconfig" \
                'CONFIG_BOBE_PROVISIONING_TRANSPORT_BLE=y'
            require_config "$sdkconfig" 'CONFIG_BT_NIMBLE_ENABLED=y'
        else
            require_config "$sdkconfig" \
                'CONFIG_BOBE_PROVISIONING_TRANSPORT_SOFTAP=y'
            reject_config "$sdkconfig" CONFIG_BT_ENABLED
        fi
    else
        reject_config "$sdkconfig" CONFIG_BOBE_ENABLE_SECURE_PROVISIONING
    fi
    if [[ "$esp_sr" == "yes" ]]; then
        require_config "$sdkconfig" 'CONFIG_BOBE_ENABLE_ESP_SR=y'
        require_config "$sdkconfig" \
            'CONFIG_SPIRAM_ALLOW_BSS_SEG_EXTERNAL_MEMORY=y'
    else
        reject_config "$sdkconfig" CONFIG_BOBE_ENABLE_ESP_SR
    fi
    if ! grep -a -q 'physical button event_id=' "$build/bobe_wb12.elf"; then
        printf 'Physical button evidence marker is missing from %s\n' \
            "$name" >&2
        return 1
    fi

    local image="$build/bobe_wb12.bin"
    local image_size
    image_size="$(wc -c < "$image" | tr -d '[:space:]')"
    if (( image_size > 0x500000 )); then
        printf '%s image exceeds the 5 MiB OTA slot: %s bytes\n' \
            "$name" "$image_size" >&2
        return 1
    fi
    printf '%s firmware: %s bytes, sha256 %s\n' \
        "$name" \
        "$image_size" \
        "$(shasum -a 256 "$image" | awk '{print $1}')"
}

build_ui_tests() {
    local build="$CHECK_ROOT/host-ui"
    cmake -S "$ROOT/host_tests/ui" -B "$build" \
        -DCMAKE_BUILD_TYPE=Debug
    cmake --build "$build" --parallel
    ctest --test-dir "$build" --output-on-failure
}

regenerate_ui_references() {
    local build="$CHECK_ROOT/host-ui"
    cmake -S "$ROOT/host_tests/ui" -B "$build" \
        -DCMAKE_BUILD_TYPE=Debug
    cmake --build "$build" --parallel --target bobe_ui_regenerate
}

build_core_policy_tests() {
    local build="$CHECK_ROOT/host-core-policy"
    local sanitized="$CHECK_ROOT/host-core-policy-sanitized"

    cmake -S "$ROOT/host_tests" -B "$build" \
        -DCMAKE_BUILD_TYPE=Debug
    cmake --build "$build" --parallel --target bobe_core_policy_tests
    ctest --test-dir "$build" -R '^bobe_core_policy_tests$' \
        --output-on-failure

    cmake -S "$ROOT/host_tests" -B "$sanitized" \
        -DCMAKE_BUILD_TYPE=Debug \
        -DBOBE_ENABLE_SANITIZERS=ON
    cmake --build "$sanitized" --parallel --target bobe_core_policy_tests
    ASAN_OPTIONS=detect_leaks=0 \
        ctest --test-dir "$sanitized" -R '^bobe_core_policy_tests$' \
        --output-on-failure
}

build_fixture() {
    "$ROOT/tools/build_pico_fixture.sh" 2>&1 |
        tee "$CHECK_ROOT/fixture-build.log"
}

check_capture() {
    "$ROOT/tools/macos/tests/run.sh" 2>&1 |
        tee "$CHECK_ROOT/capture-tests.log"
}

cd "$ROOT"
case "$ACTION" in
    all)
        ;;
    ui)
        build_ui_tests
        printf 'Deterministic host UI checks passed.\n'
        exit 0
        ;;
    ui-regenerate)
        regenerate_ui_references
        exit 0
        ;;
    core-policy)
        build_core_policy_tests
        printf 'Pure core policy and sanitizer checks passed.\n'
        exit 0
        ;;
    fixture)
        build_fixture
        printf 'Pico fixture firmware build passed.\n'
        exit 0
        ;;
    capture)
        check_capture
        printf 'macOS capture tool checks passed.\n'
        exit 0
        ;;
esac

uv run --group hil --locked python -m py_compile \
    tools/*.py \
    bobe_hil/*.py \
    bobe_observer/*.py
uv run --group hil --locked python -m unittest discover \
    -s host_tests \
    -p 'test_*.py'
uv run --group hil --group dev --locked pyright
for evaluation in .claude/skills/*/evals/evals.json; do
    uv run --group hil --locked python -m json.tool "$evaluation" >/dev/null
done
for example in \
    hil/calibration/audio.example.json \
    hil/calibration/camera.example.json \
    hil/fixture/pico2/fixture-config.example.json \
    hil/observer/observer-config.example.json \
    hil/observer/qualification.example.json \
    hil/observer/service-config.example.json \
    hil/router/network-config.example.json; do
    uv run --group hil --locked python -m json.tool "$example" >/dev/null
done
uv run --group hil --locked python -m bobe_hil scenario list >/dev/null
for script in tools/*.sh; do
    [[ -x "$script" ]] || {
        printf 'Tool script is not executable: %s\n' "$script" >&2
        exit 1
    }
    /bin/bash -n "$script"
done
sh -n hil/router/bobe-netem

cmake -S "$ROOT/host_tests" -B "$CHECK_ROOT/host" \
    -DCMAKE_BUILD_TYPE=Debug
cmake --build "$CHECK_ROOT/host" --parallel
ctest --test-dir "$CHECK_ROOT/host" --output-on-failure

cmake -S "$ROOT/host_tests" -B "$CHECK_ROOT/host-sanitized" \
    -DCMAKE_BUILD_TYPE=Debug \
    -DBOBE_ENABLE_SANITIZERS=ON
cmake --build "$CHECK_ROOT/host-sanitized" --parallel
ASAN_OPTIONS=detect_leaks=0 \
    ctest --test-dir "$CHECK_ROOT/host-sanitized" --output-on-failure

build_ui_tests
build_fixture
check_capture

build_firmware_profile \
    fallback \
    "$ROOT/sdkconfig.defaults" \
    no \
    no
build_firmware_profile \
    ptt \
    "$ROOT/sdkconfig.defaults;$ROOT/sdkconfig.provisioning.defaults" \
    yes \
    no
build_firmware_profile \
    edge \
    "$ROOT/sdkconfig.defaults;$ROOT/sdkconfig.provisioning.defaults;$ROOT/sdkconfig.edge.defaults" \
    yes \
    yes
build_firmware_profile \
    ble \
    "$ROOT/sdkconfig.defaults;$ROOT/sdkconfig.ble.defaults" \
    yes \
    no \
    ble

printf 'Host, sanitizer, fallback, PTT, edge, and BLE build checks passed. No device was flashed.\n'
