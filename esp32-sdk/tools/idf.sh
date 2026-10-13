#!/usr/bin/env bash
set -euo pipefail

SDK_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
version="${BOBE_IDF_VERSION:-6.1}"
case "$version" in 6.0.2|6.1) ;; *) printf 'Unsupported ESP-IDF version\n' >&2; exit 2;; esac
idf_dir="$SDK_ROOT/.deps/esp-idf-v$version"
if [[ ! -f "$idf_dir/export.sh" ]]; then
    printf 'Run BOBE_IDF_VERSION=%s tools/bootstrap.sh first.\n' "$version" >&2
    exit 1
fi
export IDF_TOOLS_PATH="$SDK_ROOT/.deps/espressif"
export IDF_PYTHON_ENV_PATH="$IDF_TOOLS_PATH/python_env/bobe-$version"
set +u
source "$idf_dir/export.sh" >/dev/null
set -u
exec idf.py "$@"
