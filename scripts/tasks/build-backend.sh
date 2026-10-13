#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
profile="${1:-release}"
case "$profile" in
    release) profile_args=(--release) ;;
    debug) profile_args=(--profile dev) ;;
    *)
        echo "build-backend profile must be 'debug' or 'release': $profile" >&2
        exit 1
        ;;
esac
"$ROOT/scripts/tasks/build-agent.sh"
(
    cd BoBeService
    if [[ "$(uname -s)" == Darwin ]]; then
        export MACOSX_DEPLOYMENT_TARGET=26.0
    fi
    cargo build --locked "${profile_args[@]}"
)
if [[ "$(uname -s)" == Darwin ]]; then
    ./scripts/check-macos-deployment-target.sh \
        --require-mach-o "BoBeService/target/$profile/bobe"
fi
