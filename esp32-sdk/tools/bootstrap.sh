#!/usr/bin/env bash
set -euo pipefail

SDK_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$SDK_ROOT"
version="${BOBE_IDF_VERSION:-6.1}"
case "$version" in 6.0.2|6.1) ;; *) printf 'Supported ESP-IDF versions: 6.0.2, 6.1\n' >&2; exit 2;; esac
idf_dir="$SDK_ROOT/.deps/esp-idf-v$version"
if [[ ! -d "$idf_dir/.git" ]]; then
    mkdir -p "$SDK_ROOT/.deps"
    git clone --filter=blob:none --depth 1 --branch "v$version" \
        https://github.com/espressif/esp-idf.git "$idf_dir"
fi
[[ "$(git -C "$idf_dir" describe --tags --exact-match HEAD)" == "v$version" ]]
git -C "$idf_dir" submodule update --init --recursive --depth 1
uv sync --locked
export IDF_TOOLS_PATH="$SDK_ROOT/.deps/espressif"
export IDF_PYTHON_ENV_PATH="$IDF_TOOLS_PATH/python_env/bobe-$version"
uv run --locked python "$idf_dir/tools/idf_tools.py" install --targets esp32,esp32c3,esp32c6,esp32s3
uv run --locked python - "$idf_dir" "$IDF_TOOLS_PATH" <<'PY'
import sys
sys.path.insert(0, sys.argv[1] + "/tools")
import idf_tools
idf_tools.g.idf_path = sys.argv[1]
idf_tools.g.idf_tools_path = sys.argv[2]
idf_tools.get_constraints(idf_tools.get_idf_version())
PY
if [[ ! -x "$IDF_PYTHON_ENV_PATH/bin/python" ]]; then
    uv venv --python "$SDK_ROOT/.venv/bin/python" "$IDF_PYTHON_ENV_PATH"
fi
series="${version%.*}"
[[ "$version" == 6.1 ]] && series=6.1
uv pip install --python "$IDF_PYTHON_ENV_PATH/bin/python" \
    -r "$idf_dir/tools/requirements/requirements.core.txt" \
    -c "$IDF_TOOLS_PATH/espidf.constraints.v$series.txt"
printf '%s' "$series" > "$IDF_PYTHON_ENV_PATH/idf_version.txt"
printf 'ESP-IDF %s is ready in %s\n' "$version" "$idf_dir"
