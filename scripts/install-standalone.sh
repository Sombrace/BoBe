#!/bin/sh
set -eu

prefix="${BOBE_INSTALL_PREFIX:-$HOME/.local}"
source_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
[ -x "$source_dir/bin/node" ] || {
    printf '%s\n' "Archive is missing the bundled Node or pi agent distribution: bin/node" >&2
    exit 1
}
linux_assets=
if [ "$(uname -s)" = Linux ]; then
    linux_assets="lib/libatomic.so.1 licenses/libatomic-LICENSE"
fi
for asset in \
    dist/main.js \
    assets/chat.md \
    fixtures/host-contract-v1.json \
    package.json \
    licenses/node-LICENSE \
    licenses/rg-MIT \
    licenses/fd-MIT \
    bin/rg \
    bin/fd \
    node_modules/@earendil-works/pi-coding-agent/package.json \
    node_modules/@earendil-works/pi-agent-core/package.json \
    node_modules/@earendil-works/pi-ai/package.json \
    node_modules/quickjs-wasi/quickjs.wasm \
    node_modules/@silvia-odwyer/photon-node/photon_rs_bg.wasm \
    $linux_assets; do
    [ -s "$source_dir/bin/agent/$asset" ] || {
        printf '%s\n' "Archive is missing the bundled Node or pi agent distribution: $asset" >&2
        exit 1
    }
done
version=$("$source_dir/bin/bobe" version | sed 's/^BoBe v//')
json_version() {
    sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$1" | head -n 1
}
declared_version=$(json_version "$source_dir/compatibility.json")
if [ -z "$declared_version" ]; then
    printf '%s\n' "Archive compatibility metadata has no string version." >&2
    exit 1
fi
if [ "$declared_version" != "$version" ]; then
    printf '%s\n' "Archive version $declared_version does not match executable version $version." >&2
    exit 1
fi
versions="$prefix/share/bobe/versions"
destination="$versions/$version"
staging="$versions/.$version-$$"
trap 'rm -rf "$staging"' EXIT HUP INT TERM

rm -rf "$staging"
if [ -e "$destination" ]; then
    valid_destination=true
    if [ ! -d "$destination" ] \
        || [ ! -x "$destination/bin/bobe" ] \
        || [ ! -x "$destination/bin/node" ] \
        || [ ! -f "$destination/compatibility.json" ] \
        || ! diff -qr "$source_dir/bin" "$destination/bin" >/dev/null 2>&1 \
        || ! cmp -s "$source_dir/compatibility.json" "$destination/compatibility.json"; then
        valid_destination=false
    else
        installed_version=$("$destination/bin/bobe" version 2>/dev/null | sed 's/^BoBe v//' || true)
        installed_declared_version=$(json_version "$destination/compatibility.json")
        if [ "$installed_version" != "$version" ] \
            || [ "$installed_declared_version" != "$version" ]; then
            valid_destination=false
        fi
    fi
    if [ "$valid_destination" != true ]; then
        printf '%s\n' "Refusing to activate incomplete or mismatched existing version: $destination" >&2
        exit 1
    fi
else
    mkdir -p "$staging/bin"
    install -m 0755 "$source_dir/bin/bobe" "$staging/bin/bobe"
    install -m 0755 "$source_dir/bin/node" "$staging/bin/node"
    cp -RP "$source_dir/bin/agent" "$staging/bin/agent"
    if [ -d "$source_dir/bin/licenses" ]; then
        cp -RP "$source_dir/bin/licenses" "$staging/bin/licenses"
    fi
    install -m 0644 "$source_dir/compatibility.json" "$staging/compatibility.json"
    mv "$staging" "$destination"
fi
mkdir -p "$prefix/bin"
replace_link() {
    if [ "$(uname -s)" = Darwin ]; then
        mv -fh "$1" "$2"
    else
        mv -fT "$1" "$2"
    fi
}
ln -sfn "$version" "$versions/.current-$$"
replace_link "$versions/.current-$$" "$versions/current"
ln -sfn "../share/bobe/versions/current/bin/bobe" "$prefix/bin/.bobe-$$"
replace_link "$prefix/bin/.bobe-$$" "$prefix/bin/bobe"
printf '%s\n' "Installed BoBe CLI to $prefix/bin/bobe"
printf '%s\n' "Next: $prefix/bin/bobe setup"
