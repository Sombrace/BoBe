#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin="${BOBE_TEST_BINARY:-$root/BoBeService/target/debug/bobe}"
runtime="$root/build/agent-runtime"
state="$root/build/standalone-install-check-$$"
archive="$state/archive"
prefix="$state/prefix"

[[ -x "$bin" ]] || {
    printf 'Expected compiled daemon at %s; build it or set BOBE_TEST_BINARY.\n' "$bin" >&2
    exit 1
}
"$root/scripts/check-agent-runtime.sh" "$runtime"
mkdir -m 0700 "$state"
trap 'rm -rf "$state"' EXIT

version="$("$bin" version | sed 's/^BoBe v//')"
destination="$prefix/share/bobe/versions/$version"
mkdir -p "$archive/bin" "$destination"
install -m 0755 "$bin" "$archive/bin/bobe"
install -m 0755 "$runtime/node" "$archive/bin/node"
cp -RP "$runtime/agent" "$archive/bin/agent"
install -m 0755 "$root/scripts/install-standalone.sh" "$archive/install.sh"
printf '{"schema":1,"version":"%s","database_schema":1,"database_schema_min":1,"attach_protocol_min":1,"attach_protocol_max":1,"platform":"test"}\n' \
    "$version" >"$archive/compatibility.json"
cp "$archive/compatibility.json" "$destination/compatibility.json"

expect_refusal() {
    local reason="$1"
    if BOBE_INSTALL_PREFIX="$prefix" "$archive/install.sh" >"$state/stdout" 2>"$state/stderr"; then
        printf 'Installer accepted %s\n' "$reason" >&2
        exit 1
    fi
    if ! grep -Eq 'Refusing to activate incomplete or mismatched existing version|missing the bundled Node or pi agent' \
        "$state/stderr"; then
        cat "$state/stderr" >&2
        printf 'Installer did not explain refusal: %s\n' "$reason" >&2
        exit 1
    fi
}

expect_refusal "an incomplete existing version"
[[ ! -e "$prefix/share/bobe/versions/current" && ! -e "$prefix/bin/bobe" ]]

previous="previous-$version"
mkdir -p "$prefix/share/bobe/versions/$previous/bin" "$prefix/bin"
cp "$archive/bin/bobe" "$prefix/share/bobe/versions/$previous/bin/bobe"
ln -s "$previous" "$prefix/share/bobe/versions/current"
ln -s "../share/bobe/versions/current/bin/bobe" "$prefix/bin/bobe"
expect_refusal "an incomplete candidate while another version is active"
[[ "$(readlink "$prefix/share/bobe/versions/current")" == "$previous" ]]

rm -rf "$destination"
BOBE_INSTALL_PREFIX="$prefix" "$archive/install.sh" >/dev/null
BOBE_INSTALL_PREFIX="$prefix" "$archive/install.sh" >/dev/null
[[ "$(readlink "$prefix/share/bobe/versions/current")" == "$version" ]]
[[ "$("$prefix/bin/bobe" version)" == "BoBe v$version" ]]
diff -qr "$archive/bin" "$destination/bin" >/dev/null

for artifact in \
    bin/bobe \
    bin/node \
    bin/agent/dist/main.js \
    bin/agent/assets/chat.md \
    bin/agent/bin/rg \
    bin/agent/bin/fd \
    bin/agent/node_modules/quickjs-wasi/quickjs.wasm \
    bin/agent/node_modules/@silvia-odwyer/photon-node/photon_rs_bg.wasm; do
    printf '\0' >>"$destination/$artifact"
    expect_refusal "mismatched runtime artifact $artifact"
    [[ "$(readlink "$prefix/share/bobe/versions/current")" == "$version" ]]
    [[ "$(readlink "$prefix/bin/bobe")" == "../share/bobe/versions/current/bin/bobe" ]]
    cp "$archive/$artifact" "$destination/$artifact"
done

mv "$destination/bin/agent/dist/main.js" "$state/installed-entry"
expect_refusal "a missing installed worker entry"
mv "$state/installed-entry" "$destination/bin/agent/dist/main.js"

printf 'unexpected' >"$destination/bin/agent/unexpected-asset"
expect_refusal "a stale asset absent from the archive"
rm "$destination/bin/agent/unexpected-asset"

mv "$archive/bin/node" "$state/archive-node"
expect_refusal "an archive without Node"
mv "$state/archive-node" "$archive/bin/node"
mv "$archive/bin/agent/dist/main.js" "$state/archive-entry"
expect_refusal "an archive without its worker"
mv "$state/archive-entry" "$archive/bin/agent/dist/main.js"

BOBE_INSTALL_PREFIX="$prefix" "$archive/install.sh" >/dev/null
diff -qr "$archive/bin" "$destination/bin" >/dev/null
"$root/scripts/check-agent-runtime.sh" "$destination/bin"
printf 'Standalone installer preserves and verifies the full pi runtime\n'
