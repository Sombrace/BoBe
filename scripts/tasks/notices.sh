#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
output="${1:?usage: notices.sh <output-directory> [swift-package-checkouts]}"
swift_checkouts="${2:-}"

./scripts/tasks/bootstrap-tools.sh --check

rust_notices() {
    ./.tools/bin/cargo-about generate --locked --fail \
        -c scripts/notices/about.toml -m "$1" scripts/notices/about.hbs
}

rm -rf "$output"
mkdir -p "$output"
install -m 0644 LICENSE TRADEMARKS.md "$output/"
cp -RP third_party/speech-licenses "$output/speech"
{
    printf 'Open-source Rust crates compiled into BoBe, with their licenses.\n\n'
    rust_notices BoBeService/Cargo.toml
    if [[ -n "$swift_checkouts" ]]; then
        rust_notices PairingFFI/Cargo.toml
    fi
} >"$output/THIRD_PARTY_NOTICES.txt"

if [[ -n "$swift_checkouts" ]]; then
    cp -RP third_party/app-licenses "$output/app"
    for package in "$swift_checkouts"/*/; do
        name="$(basename "$package")"
        mkdir -p "$output/swift/$name"
        find "$package" -maxdepth 1 -type f \
            \( -iname 'LICENSE*' -o -iname 'NOTICE*' -o -iname 'COPYING*' \) \
            -exec install -m 0644 {} "$output/swift/$name/" \;
        if [[ -z "$(ls -A "$output/swift/$name")" ]]; then
            echo "Swift package $name has no license file" >&2
            exit 1
        fi
    done
fi
echo "Notices written to $output"
