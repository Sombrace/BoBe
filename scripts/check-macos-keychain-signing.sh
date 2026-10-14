#!/usr/bin/env bash
set -euo pipefail

APP="${1:?usage: check-macos-keychain-signing.sh /path/to/BoBe.app}"
EXPECTED_SUFFIX="com.bobe.app.endpoint-credential"
PROFILE="$APP/Contents/embedded.provisionprofile"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
check_dir="$ROOT/build/keychain-signing-check-$$"
mkdir -p "$ROOT/build"
mkdir -m 0700 "$check_dir"
APP_ENTITLEMENTS="$check_dir/app-entitlements.plist"
PROFILE_PLIST="$check_dir/profile.plist"
trap 'rm -rf "$check_dir"' EXIT

[[ -d "$APP" ]] || { echo "App bundle not found: $APP" >&2; exit 1; }
[[ -f "$PROFILE" ]] || {
    echo "Provisioned Data Protection build is missing embedded.provisionprofile" >&2
    exit 1
}

codesign -d --entitlements :- "$APP" >"$APP_ENTITLEMENTS" 2>/dev/null
security cms -D -i "$PROFILE" >"$PROFILE_PLIST"

read_plist() {
    /usr/libexec/PlistBuddy -c "Print :$1" "$2" 2>/dev/null
}

bundle_id="$(read_plist CFBundleIdentifier "$APP/Contents/Info.plist")"
[[ "$bundle_id" == "com.bobe.app" ]] || {
    echo "Unexpected bundle identifier: $bundle_id" >&2
    exit 1
}

team_id="$(read_plist com.apple.developer.team-identifier "$APP_ENTITLEMENTS")"
application_id="$(read_plist com.apple.application-identifier "$APP_ENTITLEMENTS")"
profile_team_id="$(read_plist Entitlements:com.apple.developer.team-identifier "$PROFILE_PLIST")"
profile_application_id="$(read_plist Entitlements:com.apple.application-identifier "$PROFILE_PLIST")"
configured_group="$(read_plist BoBeEndpointCredentialAccessGroup "$APP/Contents/Info.plist")"
expected_group="${application_id%"$bundle_id"}$EXPECTED_SUFFIX"

[[ -n "$team_id" && "$team_id" == "$profile_team_id" ]] || {
    echo "App and profile team identifiers do not agree" >&2
    exit 1
}
[[ "$application_id" == "$profile_application_id" ]] || {
    echo "App and profile application identifiers do not agree" >&2
    exit 1
}
[[ "$application_id" == *".$bundle_id" ]] || {
    echo "Effective application identifier does not match bundle identifier" >&2
    exit 1
}
[[ "$configured_group" == "$expected_group" ]] || {
    echo "Expanded endpoint credential group does not match application identity" >&2
    exit 1
}

/usr/libexec/PlistBuddy -c 'Print :keychain-access-groups' "$APP_ENTITLEMENTS" \
    | grep -Fxq "    $expected_group" || {
        echo "App lacks the exact endpoint credential access group" >&2
        exit 1
    }
[[ "$(read_plist com.apple.security.get-task-allow "$APP_ENTITLEMENTS")" != true ]] || {
    echo "App requests com.apple.security.get-task-allow, which notarization rejects" >&2
    exit 1
}
profile_allows_expected_group() {
    local group
    while IFS= read -r group; do
        group="${group#"${group%%[![:space:]]*}"}"
        [[ "$group" == "$expected_group" ]] && return 0
        [[ "$group" == *'*' && "$expected_group" == "${group%'*'}"* ]] && return 0
    done < <(/usr/libexec/PlistBuddy -c 'Print :Entitlements:keychain-access-groups' "$PROFILE_PLIST")
    return 1
}
profile_allows_expected_group || {
    echo "Provisioning profile does not allow the endpoint credential access group" >&2
    exit 1
}

signature="$(codesign -d --verbose=4 "$APP" 2>&1)"
grep -q '^Authority=Developer ID Application:' <<<"$signature" || {
    echo "App is not signed by a Developer ID Application identity" >&2
    exit 1
}
grep -Fq "TeamIdentifier=$team_id" <<<"$signature" || {
    echo "Code signature team does not match effective entitlements" >&2
    exit 1
}
grep -Eq '^CodeDirectory .*flags=.*\(runtime\)' <<<"$signature" || {
    echo "Signed app does not enable the hardened runtime" >&2
    exit 1
}

for executable in "$APP/Contents/MacOS/bobe-daemon" "$APP/Contents/MacOS/node"; do
    [[ -x "$executable" ]] || { echo "Missing runtime executable: $executable" >&2; exit 1; }
    nested_entitlements="$(codesign -d --entitlements :- "$executable" 2>/dev/null)"
    if grep -Fq "$EXPECTED_SUFFIX" <<<"$nested_entitlements"; then
        echo "Nested executable has the GUI endpoint credential group: $executable" >&2
        exit 1
    fi
done
node_entitlements="$check_dir/node-entitlements.plist"
codesign -d --entitlements :- "$APP/Contents/MacOS/node" >"$node_entitlements" 2>/dev/null
for entitlement in com.apple.security.cs.allow-jit com.apple.security.cs.allow-unsigned-executable-memory; do
    [[ "$(read_plist "$entitlement" "$node_entitlements")" == true ]] || {
        echo "Node is missing required JavaScript/WebAssembly entitlement: $entitlement" >&2
        exit 1
    }
done

codesign --verify --deep --strict --verbose=2 "$APP"
echo "Provisioned Data Protection Keychain signing verified"
