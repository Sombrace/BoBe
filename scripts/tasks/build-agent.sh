#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
source "$ROOT/scripts/lib/agent-runtime.sh"

node_home="$(bobe_prepare_node "$ROOT")"
export PATH="$node_home/bin:$PATH"
export npm_config_cache="$ROOT/.tools/cache/npm"
export NODE_DISABLE_COMPILE_CACHE=1
stage="$ROOT/build/agent-runtime-$$"
work="$ROOT/build/agent-build-$$"
export TMPDIR="$work/scratch"
trap 'rm -rf "$stage" "$work"' EXIT
mkdir -p "$TMPDIR"

test -f BoBeAgent/package-lock.json || {
    echo "BoBeAgent/package-lock.json is required for a pinned agent build" >&2
    exit 1
}
mkdir -p "$work"
cp BoBeAgent/package.json BoBeAgent/package-lock.json BoBeAgent/tsconfig.json BoBeAgent/tsconfig.build.json "$work/"
cp -RP BoBeAgent/src "$work/src"
cp -RP BoBeAgent/assets "$work/assets"
cp -RP BoBeAgent/fixtures "$work/fixtures"
npm ci --prefix "$work" --include=dev --ignore-scripts --no-audit --no-fund
npm run --prefix "$work" check
if ! npm test --prefix "$work" >"$work/test.log" 2>&1; then
    cat "$work/test.log" >&2
    exit 1
fi
tail -n 9 "$work/test.log"
npm run --prefix "$work" build

mkdir -p "$stage/agent/licenses"
install -m 0755 "$node_home/bin/node" "$stage/node"
install -m 0644 "$node_home/LICENSE" "$stage/agent/licenses/node-LICENSE"
cp BoBeAgent/package.json BoBeAgent/package-lock.json "$stage/agent/"
cp -RP "$work/dist" "$stage/agent/dist"
cp -RP "$work/assets" "$stage/agent/assets"
cp -RP "$work/fixtures" "$stage/agent/fixtures"
npm ci --prefix "$stage/agent" --omit=dev --ignore-scripts --bin-links=false --no-audit --no-fund
bobe_stage_search_tools "$ROOT" "$stage/agent"
./scripts/check-agent-runtime.sh "$stage"
rm -rf "$ROOT/build/agent-runtime"
mv "$stage" "$ROOT/build/agent-runtime"
echo "Pinned Node and pi agent staged at build/agent-runtime"
