#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TASK_DIR="$ROOT/scripts/tasks"
cd "$ROOT"

if [[ "${BOBE_SKIP_TOOL_CHECK:-0}" != "1" ]]; then
    "$TASK_DIR/bootstrap-tools.sh" --check
fi
(cd BoBeMacUI && ../.tools/bin/xcodegen generate)
mkdir -p BoBeMacUI/BoBe.xcodeproj/project.xcworkspace/xcshareddata/swiftpm
cp BoBeMacUI/Package.resolved BoBeMacUI/BoBe.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
