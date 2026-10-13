#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
"$ROOT/tools/bobe_capture.sh" --build
/usr/bin/python3 "$ROOT/tools/macos/tests/test_cli.py"
