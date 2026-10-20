#!/usr/bin/env bash
set -euo pipefail

SDK_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck disable=SC1091
source "$SDK_ROOT/tools/esp-idf.lock"
version="$ESP_IDF_VERSION"
idf_dir="$SDK_ROOT/.deps/esp-idf-$ESP_IDF_DESCRIBE"
if [[ ! -f "$idf_dir/export.sh" ||
      "$(git -C "$idf_dir" rev-parse HEAD 2>/dev/null)" != "$ESP_IDF_COMMIT" ]]; then
    printf 'Run tools/bootstrap.sh first.\n' >&2
    exit 1
fi
export IDF_TOOLS_PATH="$SDK_ROOT/.deps/espressif"
export IDF_PYTHON_ENV_PATH="$IDF_TOOLS_PATH/python_env/bobe-$version"
set +u
source "$idf_dir/export.sh" >/dev/null
set -u
exec idf.py "$@"
