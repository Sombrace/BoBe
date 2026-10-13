#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TASK_DIR="$ROOT/scripts/tasks"
cd "$ROOT"

[[ $# -ge 1 ]] || { echo "Usage: $0 <version> <identity>" >&2; exit 2; }
version="$1"
identity="${2:-Developer ID Application}"

"$TASK_DIR/build-backend.sh"
"$TASK_DIR/provisioned-app-frontend.sh"
"$TASK_DIR/bundle.sh" "$version"
"$TASK_DIR/sign.sh" "$identity"
"$TASK_DIR/dmg.sh" "$version" "$identity"

echo "Release build complete: build/BoBe-${version}.dmg"
echo "Next: set NOTARIZE_KEYCHAIN_PROFILE, then run: $TASK_DIR/notarize.sh $version"
echo "   or: $TASK_DIR/notarize.sh $version /path/to/AuthKey.p8 KEY_ID ISSUER_ID"
echo "Then: $TASK_DIR/staple.sh $version"
