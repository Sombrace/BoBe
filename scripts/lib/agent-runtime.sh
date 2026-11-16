#!/usr/bin/env bash

BOBE_NODE_VERSION="26.13.1"
BOBE_PI_VERSION="1.4.0"
BOBE_RG_VERSION="15.2.0"
BOBE_FD_VERSION="10.5.2"
BOBE_LIBATOMIC_SOURCE="gcc-14"
BOBE_LIBATOMIC_VERSION="14.2.0-4ubuntu2~24.04.1"
BOBE_PATCHELF_VERSION="0.19.3"

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
        darwin-arm64) echo "aec2514ea1ceb2ba52455e9ec65986f6749a06dd759e5db2a72e6889668c9735" ;;
        darwin-x64) echo "82bb96078a24ca50b9c9a38f2379e6be6d87171fcd400fc14c8934b3bce96e2f" ;;
        linux-arm64) echo "80a330b35415382878df18d41afbd106ea89bfb3be0087d6608e289a14c5cc7d" ;;
        linux-x64) echo "518674de2b621728a5e4486f5fb3b7d2900bb3cc4aa97969edaa463db99f9778" ;;
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

bobe_fetch() {
    local url="$1" file="$2" expected="$3" download
    if [[ -f "$file" && "$(bobe_sha256 "$file")" != "$expected" ]]; then
        echo "Discarding cached download with a checksum mismatch: $file" >&2
        rm -f "$file"
    fi
    [[ -f "$file" ]] && return
    mkdir -p "$(dirname "$file")"
    download="$file.download-$$"
    if ! curl --fail --location --silent --show-error --connect-timeout 30 --speed-limit 1024 --speed-time 60 \
        --proto '=https' --proto-redir '=https' "$url" -o "$download"; then
        rm -f "$download"
        return 1
    fi
    if [[ "$(bobe_sha256 "$download")" != "$expected" ]]; then
        echo "Checksum mismatch for $url" >&2
        rm -f "$download"
        return 1
    fi
    mv "$download" "$file"
}

bobe_prepare_node() {
    local root="$1" platform archive_name archive expected node_home binary_hash
    platform="$(bobe_node_platform)" || return
    archive_name="node-v$BOBE_NODE_VERSION-$platform"
    archive="$root/.tools/cache/node/$archive_name.tar.gz"
    expected="$(bobe_node_archive_sha256 "$platform")" || return
    node_home="$root/.tools/node/$archive_name"
    bobe_fetch "https://nodejs.org/dist/v$BOBE_NODE_VERSION/$archive_name.tar.gz" "$archive" "$expected" || return
    binary_hash="$(tar -xOzf "$archive" "$archive_name/bin/node" | bobe_sha256)"
    if [[ ! -x "$node_home/bin/node" || ! -f "$node_home/lib/node_modules/npm/bin/npm-cli.js" ]] \
        || [[ "$(bobe_sha256 "$node_home/bin/node")" != "$binary_hash" ]]; then
        rm -rf "$node_home"
        mkdir -p "$root/.tools/node"
        tar -xzf "$archive" -C "$root/.tools/node"
    fi
    printf '%s\n' "$node_home"
}

bobe_stage_node() {
    local root="$1" node_home="$2" destination="$3" platform
    platform="$(bobe_node_platform)" || return
    mkdir -p "$destination/agent/licenses"
    install -m 0755 "$node_home/bin/node" "$destination/node"
    install -m 0644 "$node_home/LICENSE" "$destination/agent/licenses/node-LICENSE"
    if [[ "$platform" == linux-* ]]; then
        bobe_stage_libatomic "$root" "$platform" "$destination" || return
    fi
    [[ "$("$destination/node" --version)" == "v$BOBE_NODE_VERSION" ]] || {
        echo "Pinned Node runtime failed its version check" >&2
        return 1
    }
}

bobe_stage_libatomic() {
    local root="$1" platform="$2" destination="$3" architecture triplet machine package package_sha patchelf patchelf_sha
    case "$platform" in
        linux-arm64)
            architecture=arm64 triplet=aarch64-linux-gnu machine=aarch64
            package_sha=fdeab74e7ad8572cf69c7024e8040c7ac851f6e667d35f0d429e7a0352840f73
            patchelf_sha=2281dd071995b449b9ce7c53c71d6d56ec3a280ae1cc66b779442abf624f7c10 ;;
        linux-x64)
            architecture=amd64 triplet=x86_64-linux-gnu machine=x86_64
            package_sha=fe49cbbc7be753528380c724a8eef5f1e31dffa9221f692c5069048d81c7449d
            patchelf_sha=38861b171079e440b25ac86483284f690d0ee7921dba98b0c8fe04a9b0a11667 ;;
        *) echo "No pinned libatomic for $platform" >&2; return 1 ;;
    esac
    command -v dpkg-deb >/dev/null || {
        echo "Bundling libatomic with the Linux Node runtime needs dpkg-deb (package dpkg)" >&2
        return 1
    }
    package="$root/.tools/cache/libatomic/libatomic1_${BOBE_LIBATOMIC_VERSION}_$architecture.deb"
    patchelf="$root/.tools/cache/patchelf/patchelf-$BOBE_PATCHELF_VERSION-$machine.tar.gz"
    bobe_fetch "https://launchpad.net/ubuntu/+archive/primary/+files/${package##*/}" "$package" "$package_sha" || return
    bobe_fetch "https://github.com/NixOS/patchelf/releases/download/$BOBE_PATCHELF_VERSION/${patchelf##*/}" \
        "$patchelf" "$patchelf_sha" || return
    mkdir -p "$destination/agent/lib" "$destination/.patchelf"
    dpkg-deb --fsys-tarfile "$package" | tar -xO "./usr/lib/$triplet/libatomic.so.1.2.0" \
        >"$destination/agent/lib/libatomic.so.1" || return
    [[ -s "$destination/agent/lib/libatomic.so.1" ]] || { echo "libatomic1 package has no library" >&2; return 1; }
    chmod 0644 "$destination/agent/lib/libatomic.so.1"
    tar -xzf "$patchelf" -C "$destination/.patchelf" ./bin/patchelf || return
    "$destination/.patchelf/bin/patchelf" --set-rpath '$ORIGIN/agent/lib' "$destination/node" || return
    rm -rf "$destination/.patchelf"
    {
        printf 'lib/libatomic.so.1 is the GNU Atomic Library from GCC, unmodified from the Ubuntu\n'
        printf 'package libatomic1 %s (source package %s).\n' "$BOBE_LIBATOMIC_VERSION" "$BOBE_LIBATOMIC_SOURCE"
        printf 'It is licensed under the GNU General Public License version 3 or later with the\n'
        printf 'GCC Runtime Library Exception version 3.1, both reproduced below.\n'
        printf 'Corresponding source: https://launchpad.net/ubuntu/+source/%s/%s\n\n' \
            "$BOBE_LIBATOMIC_SOURCE" "$BOBE_LIBATOMIC_VERSION"
        cat "$root/third_party/libatomic/COPYING.RUNTIME"
        printf '\n'
        cat "$root/third_party/libatomic/COPYING3"
    } >"$destination/agent/licenses/libatomic-LICENSE" || return
    chmod 0644 "$destination/agent/licenses/libatomic-LICENSE"
}

bobe_stage_search_tools() {
    local root="$1" destination="$2" platform target tool version repo asset expected archive extracted
    platform="$(bobe_node_platform)" || return
    mkdir -p "$destination/bin" "$destination/licenses"
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
            fd:darwin-arm64) expected=81bf98a16d3d7230e4a9123fcede243d71ea31a577113476ed43f1cdf4773162 ;;
            fd:darwin-x64) expected=0f02e6000a8c896cd254cf4dccc7da1e604e77a32c6783609eed5e4cb07785e3 ;;
            fd:linux-arm64) expected=63ea52cb31586e6d98efa2f06d73d542f16c7d0c4af0f07b579aaf9714cf907a ;;
            fd:linux-x64) expected=7e2d07fa2b8807642aaec67b7e2972a9fbe4db26fcb340a7372392db42b26694 ;;
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
        bobe_fetch "https://github.com/$repo/releases/download/$version/$asset.tar.gz" "$archive" "$expected" || return
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
