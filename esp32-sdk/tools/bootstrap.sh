#!/usr/bin/env bash
set -euo pipefail

SDK_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$SDK_ROOT"
# shellcheck disable=SC1091
source "$SDK_ROOT/tools/esp-idf.lock"
version="$ESP_IDF_VERSION"
idf_dir="$SDK_ROOT/.deps/esp-idf-$ESP_IDF_DESCRIBE"
if [[ "$(git -C "$idf_dir" rev-parse -q --verify HEAD 2>/dev/null)" != "$ESP_IDF_COMMIT" ]]; then
    mkdir -p "$idf_dir"
    [[ -d "$idf_dir/.git" ]] || git -C "$idf_dir" init --quiet
    git -C "$idf_dir" remote remove origin 2>/dev/null || true
    git -C "$idf_dir" remote add origin https://github.com/espressif/esp-idf.git
    git -C "$idf_dir" fetch --depth 1 origin "$ESP_IDF_COMMIT"
    git -C "$idf_dir" checkout --quiet --detach FETCH_HEAD
fi
[[ "$(git -C "$idf_dir" rev-parse HEAD)" == "$ESP_IDF_COMMIT" ]]
git -C "$idf_dir" submodule update --init --recursive --depth 1
printf '%s\n' "$ESP_IDF_DESCRIBE" > "$idf_dir/version.txt"
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
uv pip install --python "$IDF_PYTHON_ENV_PATH/bin/python" \
    -r "$idf_dir/tools/requirements/requirements.core.txt" \
    -c "$IDF_TOOLS_PATH/espidf.constraints.v$version.txt"
printf '%s' "$version" > "$IDF_PYTHON_ENV_PATH/idf_version.txt"
printf 'ESP-IDF %s (%s) is ready in %s\n' "$ESP_IDF_DESCRIBE" "$ESP_IDF_COMMIT" "$idf_dir"
