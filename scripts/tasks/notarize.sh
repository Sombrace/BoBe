#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

usage() {
    echo "Usage: $0 <version> [<key-path> <key-id> <issuer>]" >&2
    echo "Without an App Store Connect API key, set NOTARIZE_KEYCHAIN_PROFILE to a notarytool Keychain profile." >&2
    exit 2
}

[[ $# -eq 1 || $# -eq 4 ]] || usage
version="$1"
if [[ $# -eq 4 ]]; then
    [[ -n "$2" && -n "$3" && -n "$4" ]] || usage
    credentials=(--key "$2" --key-id "$3" --issuer "$4")
else
    : "${NOTARIZE_KEYCHAIN_PROFILE:?Set NOTARIZE_KEYCHAIN_PROFILE to a notarytool Keychain profile, or pass an API key}"
    credentials=(--keychain-profile "$NOTARIZE_KEYCHAIN_PROFILE")
fi

result="$(xcrun notarytool submit "build/BoBe-${version}.dmg" "${credentials[@]}" --wait --output-format json)"
status="$(plutil -extract status raw -o - - <<<"$result")"
submission="$(plutil -extract id raw -o - - <<<"$result")"
if [[ "$status" != "Accepted" ]]; then
    echo "Notarization of build/BoBe-${version}.dmg is $status (submission $submission)." >&2
    echo "Read the reasons with: xcrun notarytool log $submission and the same credentials" >&2
    exit 1
fi
echo "Notarization accepted (submission $submission)"
