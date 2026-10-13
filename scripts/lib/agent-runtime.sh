#!/usr/bin/env bash

BOBE_NODE_VERSION="22.22.0"
BOBE_PI_VERSION="1.0.3"
BOBE_RG_VERSION="15.1.0"
BOBE_FD_VERSION="10.3.0"

bobe_node_platform() {
    local system architecture
    system="$(uname -s)"
    architecture="$(uname -m)"
    case "$system" in
        Darwin) system="darwin" ;;
        Linux) system="linux" ;;
        *) echo "Unsupported Node platform: $system" >&2; return 1 ;;
    esac
    case "$architecture" in
        arm64|aarch64) architecture="arm64" ;;
        x86_64|amd64) architecture="x64" ;;
        *) echo "Unsupported Node architecture: $architecture" >&2; return 1 ;;
    esac
    printf '%s-%s\n' "$system" "$architecture"
}

bobe_node_archive_sha256() {
    case "$1" in
        darwin-arm64) echo "5ed4db0fcf1eaf84d91ad12462631d73bf4576c1377e192d222e48026a902640" ;;
        darwin-x64) echo "5ea50c9d6dea3dfa3abb66b2656f7a4e1c8cef23432b558d45fb538c7b5dedce" ;;
        linux-arm64) echo "25ba95dfb96871fa2ef977f11f95ea90818c8fa15c0f2110771db08d4ba423be" ;;
        linux-x64) echo "c33c39ed9c80deddde77c960d00119918b9e352426fd604ba41638d6526a4744" ;;
        *) echo "No pinned Node checksum for $1" >&2; return 1 ;;
    esac
}

bobe_sha256() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$@" | awk '{print $1}'
    else
        sha256sum "$@" | awk '{print $1}'
    fi
}

bobe_prepare_node() {
    local root="$1" platform archive_name archive expected actual node_home
    platform="$(bobe_node_platform)" || return
    archive_name="node-v$BOBE_NODE_VERSION-$platform"
    archive="$root/.tools/cache/node/$archive_name.tar.gz"
    expected="$(bobe_node_archive_sha256 "$platform")" || return
    node_home="$root/.tools/node/$archive_name"
    mkdir -p "$root/.tools/cache/node" "$root/.tools/node"
    if [[ -f "$archive" && "$(bobe_sha256 "$archive")" != "$expected" ]]; then
        rm -f "$archive"
    fi
    if [[ ! -f "$archive" ]]; then
        local download="$archive.download-$$"
        if ! curl --fail --location --silent --show-error --proto '=https' \
            "https://nodejs.org/dist/v$BOBE_NODE_VERSION/$archive_name.tar.gz" -o "$download"; then
            rm -f "$download"
            return 1
        fi
        actual="$(bobe_sha256 "$download")"
        if [[ "$actual" != "$expected" ]]; then
            echo "Node $BOBE_NODE_VERSION archive checksum mismatch for $platform" >&2
            rm -f "$download"
            return 1
        fi
        mv "$download" "$archive"
    fi
    local binary_hash
    binary_hash="$(tar -xOzf "$archive" "$archive_name/bin/node" | bobe_sha256)"
    if [[ ! -x "$node_home/bin/node" || ! -f "$node_home/lib/node_modules/npm/bin/npm-cli.js" ]] \
        || [[ "$(bobe_sha256 "$node_home/bin/node")" != "$binary_hash" ]]; then
        rm -rf "$node_home"
        tar -xzf "$archive" -C "$root/.tools/node"
    fi
    [[ "$("$node_home/bin/node" --version)" == "v$BOBE_NODE_VERSION" ]] || {
        echo "Pinned Node runtime failed its version check" >&2
        return 1
    }
    printf '%s\n' "$node_home"
}

bobe_stage_search_tools() {
    local root="$1" destination="$2" platform target tool version repo asset expected archive extracted
    platform="$(bobe_node_platform)" || return
    mkdir -p "$root/.tools/cache/pi-tools" "$destination/bin" "$destination/licenses"
    for tool in rg fd; do
        case "$platform" in
            darwin-arm64) target=aarch64-apple-darwin ;;
            darwin-x64) target=x86_64-apple-darwin ;;
            linux-arm64)
                if [[ "$tool" == rg ]]; then target=aarch64-unknown-linux-gnu
                else target=aarch64-unknown-linux-musl; fi ;;
            linux-x64) target=x86_64-unknown-linux-musl ;;
        esac
        case "$tool:$platform" in
            rg:darwin-arm64) expected=378e973289176ca0c6054054ee7f631a065874a352bf43f0fa60ef079b6ba715 ;;
            rg:darwin-x64) expected=64811cb24e77cac3057d6c40b63ac9becf9082eedd54ca411b475b755d334882 ;;
            rg:linux-arm64) expected=2b661c6ef508e902f388e9098d9c4c5aca72c87b55922d94abdba830b4dc885e ;;
            rg:linux-x64) expected=1c9297be4a084eea7ecaedf93eb03d058d6faae29bbc57ecdaf5063921491599 ;;
            fd:darwin-arm64) expected=0570263812089120bc2a5d84f9e65cd0c25e4a4d724c80075c357239c74ae904 ;;
            fd:darwin-x64) expected=50d30f13fe3d5914b14c4fff5abcbd4d0cdab4b855970a6956f4f006c17117a3 ;;
            fd:linux-arm64) expected=996b9b1366433b211cb3bbedba91c9dbce2431842144d925428ead0adf32020b ;;
            fd:linux-x64) expected=2b6bfaae8c48f12050813c2ffe1884c61ea26e750d803df9c9114550a314cd14 ;;
            *) echo "Unsupported native pi tool platform: $platform" >&2; return 1 ;;
        esac
        if [[ "$tool" == rg ]]; then
            version="$BOBE_RG_VERSION"
            repo=BurntSushi/ripgrep
            asset="ripgrep-$version-$target"
        else
            version="v$BOBE_FD_VERSION"
            repo=sharkdp/fd
            asset="fd-$version-$target"
        fi
        archive="$root/.tools/cache/pi-tools/$asset.tar.gz"
        if [[ ! -f "$archive" ]]; then
            if ! curl --fail --location --silent --show-error --max-time 120 --proto '=https' \
                "https://github.com/$repo/releases/download/$version/$asset.tar.gz" -o "$archive.download"; then
                rm -f "$archive.download"
                return 1
            fi
            if [[ "$(bobe_sha256 "$archive.download")" != "$expected" ]]; then
                rm -f "$archive.download"
                echo "Native pi tool checksum mismatch: $asset" >&2
                return 1
            fi
            mv "$archive.download" "$archive"
        fi
        [[ "$(bobe_sha256 "$archive")" == "$expected" ]] || {
            echo "Cached native pi tool checksum mismatch: $archive" >&2
            return 1
        }
        extracted="$destination/.extract-$tool"
        mkdir "$extracted"
        tar -xzf "$archive" -C "$extracted" --strip-components=1
        install -m 0755 "$extracted/$tool" "$destination/bin/$tool"
        [[ -f "$extracted/LICENSE-MIT" ]] || {
            echo "Missing native tool license: $asset" >&2
            return 1
        }
        install -m 0644 "$extracted/LICENSE-MIT" "$destination/licenses/$tool-MIT"
        rm -rf "$extracted"
    done
}

bobe_sign_agent_modules() {
    local agent="$1"
    shift
    [[ -d "$agent" ]] || { echo "Missing agent distribution: $agent" >&2; return 1; }
    while IFS= read -r -d '' artifact; do
        if [[ "$(file -b "$artifact")" == *Mach-O* ]]; then
            codesign "$@" "$artifact"
        fi
    done < <(find "$agent" -type f -print0)
}
