#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
[[ $# -ge 1 ]] || { echo "Usage: $0 <archive> <private-key-file>" >&2; exit 2; }
if [[ "$(uname -s)" != Darwin ]]; then
    echo "Sparkle's sign_update runs only on macOS. Copy the archive to the release Mac and sign it there." >&2
    exit 1
fi
archive="${1}"
private_key_file="${2:-}"
SPARKLE_BIN="BoBeMacUI/.build/artifacts/sparkle/Sparkle/bin"
if [[ ! -x "$SPARKLE_BIN/sign_update" ]]; then
    echo "Sparkle tools not found. Run: cd BoBeMacUI && swift package resolve" >&2
    exit 1
fi
if [[ ! -f "$archive" ]]; then
    echo "Standalone archive not found: $archive" >&2
    exit 1
fi
if [[ -n "$private_key_file" && "$private_key_file" != "-" && ! -f "$private_key_file" ]]; then
    echo "Sparkle private key file not found: $private_key_file" >&2
    exit 1
fi
manifest="$(python3 scripts/create-standalone-update-manifest.py "$archive")"
signature="$manifest.sig"
args=(-p)
if [[ -n "$private_key_file" ]]; then
    args+=(--ed-key-file "$private_key_file")
fi
"$SPARKLE_BIN/sign_update" "${args[@]}" "$manifest" >"$signature.tmp"
tr -d '\r\n' <"$signature.tmp" >"$signature"
rm -f "$signature.tmp"
[[ -s "$signature" ]] || {
    echo "Standalone manifest signature is empty" >&2
    exit 1
}
echo "$manifest"
echo "$signature"
