#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/lib/agent-runtime.sh"
export NODE_DISABLE_COMPILE_CACHE=1
check_work="$ROOT/build/agent-check-$$"
mkdir -p "$ROOT/build"
mkdir -m 0700 "$check_work"
export TMPDIR="$check_work"
trap 'rm -rf "$check_work"' EXIT
runtime="${1:-$ROOT/build/agent-runtime}"
runtime="$(cd "$runtime" && pwd)"
agent="${2:-$runtime/agent}"
agent="$(cd "$agent" && pwd)"
[[ -x "$runtime/node" ]] || { echo "Missing bundled Node executable: $runtime/node" >&2; exit 1; }
for asset in dist/main.js assets/chat.md fixtures/host-contract-v1.json licenses/node-LICENSE \
    licenses/rg-MIT licenses/fd-MIT bin/rg bin/fd; do
    [[ -s "$agent/$asset" ]] || {
        echo "Missing agent runtime asset: $agent/$asset" >&2
        exit 1
    }
done
while IFS= read -r -d '' path; do
    case "${path##*/}" in
        dist|assets|fixtures|licenses|bin|lib|node_modules|package.json|package-lock.json) ;;
        *)
            echo "Unexpected state or file in agent distribution: $path" >&2
            exit 1
            ;;
    esac
done < <(find "$agent" -mindepth 1 -maxdepth 1 -print0)
if find "$agent" -type l -print -quit | grep -q .; then
    echo "Agent runtime contains a symlink unsupported by the signed standalone archive format" >&2
    exit 1
fi
[[ "$("$runtime/node" --version)" == "v$BOBE_NODE_VERSION" ]] || {
    echo "Agent distribution does not contain pinned Node $BOBE_NODE_VERSION" >&2
    exit 1
}
if [[ "$(uname -s)" == Linux ]]; then
    [[ -s "$agent/licenses/libatomic-LICENSE" ]] || {
        echo "Missing agent runtime asset: $agent/licenses/libatomic-LICENSE" >&2
        exit 1
    }
    [[ "$(env -u LD_LIBRARY_PATH -u LD_PRELOAD ldd "$runtime/node" | awk '$1 == "libatomic.so.1" {print $3}')" \
        -ef "$agent/lib/libatomic.so.1" ]] || {
        echo "Bundled Node does not load the bundled libatomic from $agent/lib" >&2
        exit 1
    }
fi
"$runtime/node" --check "$agent/dist/main.js"
[[ "$("$agent/bin/rg" --version | head -n 1 | awk '{print $2}')" == "$BOBE_RG_VERSION" ]] || {
    echo "Bundled ripgrep is not the pinned version" >&2
    exit 1
}
[[ "$("$agent/bin/fd" --version)" == "fd $BOBE_FD_VERSION" ]] || {
    echo "Bundled fd is not the pinned version" >&2
    exit 1
}
(
    cd "$agent"
    "$runtime/node" --input-type=module - "$BOBE_PI_VERSION" <<'JS'
import assert from "node:assert/strict";
import { readFileSync, existsSync } from "node:fs";
import { glob } from "node:fs/promises";
import { join } from "node:path";
import { Agent } from "@earendil-works/pi-agent-core";
import { createModels } from "@earendil-works/pi-ai";
import { openaiProvider } from "@earendil-works/pi-ai/providers/openai";
import { createAgentSession } from "@earendil-works/pi-coding-agent";

const version = process.argv[2];
const manifest = JSON.parse(readFileSync("package.json", "utf8"));
const contract = JSON.parse(readFileSync("fixtures/host-contract-v1.json", "utf8"));
assert.equal(contract.harness, "pi");
assert.equal(contract.harness_version, version);
assert.equal(contract.protocol_version, 1);
for (const name of ["pi-agent-core", "pi-ai", "pi-coding-agent", "pi-mcp"]) {
    const id = `@earendil-works/${name}`;
    const path = join("node_modules", id);
    const installed = JSON.parse(readFileSync(join(path, "package.json"), "utf8"));
    if (name !== "pi-agent-core") assert.equal(manifest.dependencies[id], version, `${id} declared pin`);
    assert.equal(installed.version, version, `${id} installed pin`);
    assert.ok(existsSync(join(path, installed.main)), `${id} entry`);
}
assert.equal(typeof Agent, "function");
assert.equal(typeof createAgentSession, "function");
assert.equal(typeof createModels, "function");
assert.ok(openaiProvider().getModels().length > 0, "pi model catalog assets are available");
assert.ok(!existsSync("node_modules/typescript"), "development compiler is not shipped");
assert.ok(!existsSync("dist/test-support.js"), "test sources are not shipped");
const coding = "node_modules/@earendil-works/pi-coding-agent";
for (const asset of ["dist/modes/interactive/theme/dark.json", "dist/modes/interactive/theme/light.json",
    "dist/core/export-html/template.html"]) {
    assert.ok(readFileSync(join(coding, asset)).length > 0, `missing pi asset: ${asset}`);
}
const wasm = await Array.fromAsync(glob("node_modules/**/*.wasm"));
assert.ok(wasm.length > 0, "pi WebAssembly assets are shipped");
for (const asset of wasm) await WebAssembly.compile(readFileSync(asset));
console.log(`Agent distribution verified: pi ${version}, ${wasm.length} WebAssembly asset(s)`);
JS
)
