#!/usr/bin/env bash
set -euo pipefail
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
SDK_ROOT="${ESP_BOBE_SDK_PATH:-$(cd "$PROJECT_ROOT/../.." && pwd -P)}"
exec "$SDK_ROOT/tools/idf.sh" -C "$PROJECT_ROOT" "$@"
