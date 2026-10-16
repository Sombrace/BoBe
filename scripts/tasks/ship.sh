#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TASK_DIR="$ROOT/scripts/tasks"
cd "$ROOT"
[[ $# -ge 1 ]] || { echo "Usage: $0 <version> <identity>" >&2; exit 2; }
version="${1}"
identity="${2:-Developer ID Application}"
tag="v$version"
verify_release_state() {
    if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
        echo "Refusing to ship from a dirty worktree" >&2
        exit 1
    fi
    if ! git show-ref --verify --quiet "refs/tags/$tag"; then
        echo "Refusing to ship without local tag $tag" >&2
        exit 1
    fi
    if [[ "$(git rev-parse HEAD)" != "$(git rev-parse "$tag^{commit}")" ]]; then
        echo "Refusing to ship: HEAD does not match $tag" >&2
        exit 1
    fi
    "$ROOT/scripts/check-release-version.sh" "$version"
}
verify_release_state

echo "=== Clean ==="
"$TASK_DIR/clean.sh"

echo "=== Resolve dependencies ==="
(cd BoBeService && cargo fetch --locked)
(cd BoBeMacUI && swift package resolve)
verify_release_state

echo "=== Build + Bundle + Sign + DMG ==="
"$TASK_DIR/release.sh" "${version}" "${identity}"
"$ROOT/scripts/check-release-version.sh" "$version" --daemon build/BoBe.app/Contents/MacOS/bobe-daemon

echo "=== Notarize ==="
"$TASK_DIR/notarize.sh" "${version}"

echo "=== Staple ==="
"$TASK_DIR/staple.sh" "${version}"

echo "=== Sparkle ZIP ==="
"$TASK_DIR/sparkle-zip.sh" "${version}"

echo ""
echo "=== Ship complete ==="
echo "DMG:         build/BoBe-${version}.dmg (signed + notarized + stapled)"
echo "Sparkle ZIP: build/BoBe-${version}.zip"
echo ""
echo "Next: $TASK_DIR/sparkle-sign-update.sh ${version}"
echo "Then: upload DMG + ZIP + update appcast.xml"
