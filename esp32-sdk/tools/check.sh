#!/usr/bin/env bash
set -euo pipefail

SDK_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXAMPLE="$SDK_ROOT/examples/wb12"

cd "$SDK_ROOT"
uv sync --locked --all-groups
uv run --no-sync python -m unittest discover -s tests/python -v
uv run --no-sync pyright
cmake -S "$SDK_ROOT" -B "$SDK_ROOT/.tmp/portable" \
    -DCMAKE_BUILD_TYPE=Debug -DESP_BOBE_SANITIZE=ON
cmake --build "$SDK_ROOT/.tmp/portable" --parallel
ASAN_OPTIONS=detect_leaks=0 ctest --test-dir "$SDK_ROOT/.tmp/portable" --output-on-failure

if [[ ! -d "$EXAMPLE/managed_components/espressif__cjson" ]]; then
    printf 'Build examples/wb12 once to resolve its pinned dependencies.\n' >&2
    exit 1
fi

cd "$EXAMPLE"
uv run --locked --group hil --group dev python -m unittest discover -s host_tests -p 'test_*.py'
uv run --locked --group hil --group dev pyright

cmake -S "$EXAMPLE/host_tests" -B "$SDK_ROOT/.tmp/host" \
    -DCMAKE_BUILD_TYPE=Debug -DBOBE_ENABLE_SANITIZERS=ON
cmake --build "$SDK_ROOT/.tmp/host" --parallel
ASAN_OPTIONS=detect_leaks=0 ctest --test-dir "$SDK_ROOT/.tmp/host" --output-on-failure

cmake -S "$SDK_ROOT/tests" -B "$SDK_ROOT/.tmp/contracts"
cmake --build "$SDK_ROOT/.tmp/contracts" --parallel
ASAN_OPTIONS=detect_leaks=0 ctest --test-dir "$SDK_ROOT/.tmp/contracts" --output-on-failure

"$EXAMPLE/tools/check.sh" ui
printf 'SDK tooling, portable libraries, contracts, WB-12 host/type/visual checks passed. No hardware was exercised.\n'
