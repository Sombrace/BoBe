#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TASK_DIR="$ROOT/scripts/tasks"
cd "$ROOT"
"$TASK_DIR/build-backend.sh" debug
cd BoBeService
exec env BOBE_NODE_PATH="$ROOT/build/agent-runtime/node" \
    BOBE_AGENT_ENTRY="$ROOT/build/agent-runtime/agent/dist/main.js" \
    cargo run --locked -- serve
