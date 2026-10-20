#!/usr/bin/env bash

BOBE_NODE_VERSION="24.22.0"
BOBE_PI_VERSION="1.3.0"
BOBE_RG_VERSION="15.2.0"
BOBE_FD_VERSION="10.5.0"

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
        darwin-arm64) echo "6a9724d9b50a5f22b4816cded441b5a8d9ddae546191a5e5b7efa5a5e2ee26d5" ;;
        darwin-x64) echo "495e3f273c645f3c4677d438304cdfc701729ea5dec5728f5c8e76a2dece4b91" ;;
        linux-arm64) echo "b7e7991d5407815adfa74a2a9cd4db683552c8ce32ff4a295c22e972849c3f5d" ;;
        linux-x64) echo "28ea0a438de9a13bd7891fd758eee6c7819ee8af5442dfd8d45bd65e56d4228c" ;;
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
    local root="$1" destination="$2" platform target tool version repo asset expected archive download extracted
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
            rg:darwin-arm64) expected=3750b2e93f37e0c692657da574d7019a101c0084da05a790c83fd335bad973e4 ;;
            rg:darwin-x64) expected=af7825fcc69a2afc7a7aea55fc9af90e26421d8f20fe59df32e233c0b8a231c1 ;;
            rg:linux-arm64) expected=a740b91c82eaf9914cfedd353572f2791cbe0162c84101ee0951058f4dcbc90d ;;
            rg:linux-x64) expected=33e15bcf1624b25cdd2a55813a47a2f95dbe126268203e76aa6a585d1e7b149c ;;
            fd:darwin-arm64) expected=b67e1836c468e42e411984b56e52fa7abec08c2bd22c867398e7cc134aac5e12 ;;
            fd:darwin-x64) expected=7e31028c62c6955877735d0406807aa484c2a5e6f86235a59e26c29c301da590 ;;
            fd:linux-arm64) expected=d76c4317f7d5dba69f8a2a15856c90c777e7f0dd4e85f0de8c76de6992c374d4 ;;
            fd:linux-x64) expected=761c72dc8e120d85b22292063be8a796e2eeb20eb3e4f38b8fa2343ccf3514a7 ;;
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
        if [[ -f "$archive" && "$(bobe_sha256 "$archive")" != "$expected" ]]; then
            echo "Discarding cached native pi tool with a checksum mismatch: $archive" >&2
            rm -f "$archive"
        fi
        if [[ ! -f "$archive" ]]; then
            download="$archive.download-$$"
            if ! curl --fail --location --silent --show-error --max-time 120 --proto '=https' \
                "https://github.com/$repo/releases/download/$version/$asset.tar.gz" -o "$download"; then
                rm -f "$download"
                return 1
            fi
            if [[ "$(bobe_sha256 "$download")" != "$expected" ]]; then
                rm -f "$download"
                echo "Native pi tool checksum mismatch: $asset" >&2
                return 1
            fi
            mv "$download" "$archive"
        fi
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
