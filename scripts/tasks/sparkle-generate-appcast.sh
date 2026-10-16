#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
archives_dir="${1:-build/sparkle}"
download_url_prefix="${2:-}"
ed_key_file="${3:-}"
link="${4:-}"
sparkle_key_file="${ed_key_file}"
SPARKLE_BIN="BoBeMacUI/.build/artifacts/sparkle/Sparkle/bin"
if [[ ! -x "$SPARKLE_BIN/generate_appcast" ]]; then
    echo "Sparkle tools not found. Run: cd BoBeMacUI && swift package resolve"
    exit 1
fi
mkdir -p "$archives_dir"
if [[ -n "$sparkle_key_file" && "$sparkle_key_file" != "-" && ! -f "$sparkle_key_file" ]]; then
    echo "Sparkle private key file not found: $sparkle_key_file"
    exit 1
fi
shopt -s nullglob nocaseglob
archives=("$archives_dir"/*.zip)
other_archives=(
    "$archives_dir"/*.dmg
    "$archives_dir"/*.tar
    "$archives_dir"/*.tar.*
    "$archives_dir"/*.tgz
    "$archives_dir"/*.tbz
    "$archives_dir"/*.txz
    "$archives_dir"/*.aar
    "$archives_dir"/*.yaa
)
shopt -u nullglob nocaseglob
if (( ${#other_archives[@]} > 0 )); then
    echo "Only the app ZIP from sparkle-zip.sh may be staged in $archives_dir; remove: ${other_archives[*]}"
    exit 1
fi
if (( ${#archives[@]} == 0 )); then
    echo "No staged Sparkle ZIP found in $archives_dir"
    echo "Stage the app ZIP from sparkle-zip.sh there before running generate_appcast"
    exit 1
fi
ARGS=()
if [[ -n "$download_url_prefix" ]]; then
    if [[ "$download_url_prefix" != */ ]]; then
        download_url_prefix="$download_url_prefix/"
    fi
    ARGS+=(--download-url-prefix "$download_url_prefix")
fi
if [[ -n "$link" ]]; then
    ARGS+=(--link "$link")
fi
if [[ -n "$sparkle_key_file" ]]; then
    ARGS+=(--ed-key-file "$sparkle_key_file")
fi
"$SPARKLE_BIN/generate_appcast" "${ARGS[@]}" "$archives_dir"
version="$(python3 scripts/staged-app-version.py "$archives_dir")"
./scripts/check-macos-deployment-target.sh \
    --appcast "$archives_dir/appcast.xml" \
    --version "$version"
echo "Generated appcast at $archives_dir/appcast.xml"
