#!/usr/bin/env bash

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCK="$ROOT/tools/sigrok-cli.lock"
DEPS="$ROOT/.deps"
DESTINATION="$DEPS/sigrok-cli-arm64"
WORK=""
MOUNT=""

cleanup() {
    if [[ -n "$MOUNT" ]]; then
        hdiutil detach "$MOUNT" -quiet 2>/dev/null || true
    fi
    if [[ -n "$WORK" && -d "$WORK" ]]; then
        chmod -R u+w "$WORK" 2>/dev/null || true
        rm -rf "$WORK"
    fi
}
trap cleanup EXIT

fail() {
    printf '%s\n' "$1" >&2
    exit 1
}

read_lock() {
    local expected=36
    [[ -f "$LOCK" && ! -L "$LOCK" ]] ||
        fail "sigrok lock is unavailable or unsafe"
    [[ "$(wc -l <"$LOCK" | tr -d ' ')" == "$expected" ]] ||
        fail "sigrok lock field count is invalid"
    while IFS='=' read -r key value; do
        case "$key" in
            SIGROK_LOCK_SCHEMA|PKGCONF_VERSION|PKGCONF_URL|PKGCONF_SHA256|\
            LIBZIP_VERSION|LIBZIP_URL|LIBZIP_SHA256|LIBSIGROK_VERSION|\
            LIBSIGROK_URL|LIBSIGROK_SHA256|SIGROK_CLI_VERSION|SIGROK_CLI_URL|\
            SIGROK_CLI_SHA256|FIRMWARE_DMG_URL|FIRMWARE_DMG_SIZE|\
            FIRMWARE_DMG_SHA256|FX2LAFW_SALEAE_LOGIC_SIZE|\
            FX2LAFW_SALEAE_LOGIC_SHA256|BUILD_GLIB_VERSION|\
            BUILD_LIBUSB_VERSION|BUILD_GETTEXT_VERSION|BUILD_PCRE2_VERSION)
                printf -v "$key" '%s' "$value"
                ;;
            BUILD_GLIB_DYLIB_SIZE|BUILD_GLIB_DYLIB_SHA256|\
            BUILD_LIBUSB_DYLIB_SIZE|BUILD_LIBUSB_DYLIB_SHA256|\
            BUILD_GETTEXT_DYLIB_SIZE|BUILD_GETTEXT_DYLIB_SHA256|\
            BUILD_PCRE2_DYLIB_SIZE|BUILD_PCRE2_DYLIB_SHA256)
                printf -v "$key" '%s' "$value"
                ;;
            BUILD_GLIB_TREE_SHA256|BUILD_LIBUSB_TREE_SHA256|\
            BUILD_GETTEXT_TREE_SHA256|BUILD_PCRE2_TREE_SHA256|\
            BOOTSTRAP_SHA256|USB_IDENTITY_SOURCE_SHA256)
                printf -v "$key" '%s' "$value"
                ;;
            *)
                fail "sigrok lock contains an unknown field"
                ;;
        esac
    done <"$LOCK"
    [[ "${SIGROK_LOCK_SCHEMA:-}" == 1 ]] ||
        fail "sigrok lock schema is unsupported"
}

sha256_file() {
    /usr/bin/shasum -a 256 "$1" | /usr/bin/awk '{print $1}'
}

hash_tree() {
    local root="$1"
    shift
    (
        cd "$root"
        for path in "$@"; do
            [[ -d "$path" && ! -L "$path" ]] ||
                fail "native sigrok build input tree is unsafe"
            /usr/bin/find "$path" -type f -print
        done |
            LC_ALL=C /usr/bin/sort |
            while IFS= read -r path; do
                printf '%s  %s\n' "$(sha256_file "$path")" "$path"
            done
    ) | /usr/bin/shasum -a 256 | /usr/bin/awk '{print $1}'
}

download() {
    local url="$1"
    local digest="$2"
    local output="$3"
    /usr/bin/curl -fL --retry 2 --connect-timeout 15 --max-time 180 \
        "$url" -o "$output"
    [[ "$(sha256_file "$output")" == "$digest" ]] ||
        fail "downloaded sigrok input hash does not match"
}

require_formula() {
    local formula="$1"
    local expected="$2"
    local actual
    actual="$(brew list --versions "$formula" 2>/dev/null || true)"
    [[ "$actual" == "$formula $expected" ]] ||
        fail "existing $formula build input must be exactly $expected"
}

patch_dependency() {
    local file="$1"
    local old="$2"
    local replacement="$3"
    local dependency
    dependency="$(
        /usr/bin/otool -L "$file" |
            /usr/bin/awk 'NR > 1 {print $1}' |
            while IFS= read -r candidate; do
                if [[ "$candidate" == "$old" ||
                      "${candidate##*/}" == "${old##*/}" ]]; then
                    printf '%s\n' "$candidate"
                    break
                fi
            done
    )"
    [[ -n "$dependency" ]] ||
        fail "required sigrok runtime dependency is unavailable for patching"
    /usr/bin/install_name_tool -change "$dependency" "$replacement" "$file"
}

audit_runtime() {
    local runtime="$1"
    local file
    while IFS= read -r file; do
        if /usr/bin/file "$file" | /usr/bin/grep -q 'Mach-O'; then
            [[ "$(/usr/bin/lipo -archs "$file")" == arm64 ]] ||
                fail "sigrok runtime contains a non-arm64 Mach-O file"
            if /usr/bin/otool -L "$file" |
                /usr/bin/awk 'NR > 1 {print $1}' |
                /usr/bin/grep -Ev \
                    '^(@executable_path/|@loader_path/|/usr/lib/|/System/Library/)' \
                    >/dev/null; then
                fail "sigrok runtime contains an external dynamic dependency"
            fi
        fi
    done < <(/usr/bin/find "$runtime" -type f -print)
}

verify_runtime_manifest() {
    local runtime="$1"
    (
        cd "$runtime"
        [[ -z "$(/usr/bin/find . -type l -print -quit)" ]] &&
            /usr/bin/shasum -a 256 -c MANIFEST.sha256 >/dev/null &&
            /usr/bin/diff -u \
                <(/usr/bin/awk '{print $2}' MANIFEST.sha256 |
                    LC_ALL=C /usr/bin/sort) \
                <(/usr/bin/find . -type f ! -name MANIFEST.sha256 -print |
                    LC_ALL=C /usr/bin/sort) \
                >/dev/null
    )
}

read_lock
[[ "$(sha256_file "$ROOT/tools/bootstrap_sigrok_cli.sh")" == \
    "$BOOTSTRAP_SHA256" &&
   "$(sha256_file "$ROOT/tools/sigrok_usb_identity.c")" == \
    "$USB_IDENTITY_SOURCE_SHA256" ]] ||
    fail "project-local sigrok build inputs do not match the lock"
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] ||
    fail "native sigrok bootstrap requires macOS on Apple silicon"
[[ $EUID -ne 0 ]] || fail "sigrok bootstrap must not run as root"
[[ ! -L "$DEPS" ]] || fail "repository dependency directory is unsafe"
mkdir -p "$DEPS"
[[ -d "$DEPS" && "$DEPS" == "$ROOT/.deps" ]] ||
    fail "repository dependency directory is invalid"

if [[ -e "$DESTINATION" || -L "$DESTINATION" ]]; then
    if [[ ! -L "$DESTINATION" &&
          -d "$DESTINATION" &&
          -x "$DESTINATION/bin/sigrok-cli" &&
          ! -L "$DESTINATION/bin/sigrok-cli" &&
          -x "$DESTINATION/bin/sigrok-usb-identity" &&
          ! -L "$DESTINATION/bin/sigrok-usb-identity" &&
          -f "$DESTINATION/MANIFEST.sha256" &&
          ! -L "$DESTINATION/MANIFEST.sha256" &&
          -f "$DESTINATION/sigrok-cli.lock" &&
          ! -L "$DESTINATION/sigrok-cli.lock" ]] &&
       /usr/bin/cmp -s "$LOCK" "$DESTINATION/sigrok-cli.lock" &&
       verify_runtime_manifest "$DESTINATION"; then
        audit_runtime "$DESTINATION"
        "$DESTINATION/bin/sigrok-cli" --version | /usr/bin/head -1
        exit 0
    fi
    [[ "$DESTINATION" == "$ROOT/.deps/sigrok-cli-arm64" &&
       -d "$DESTINATION" && ! -L "$DESTINATION" ]] ||
        fail "stale sigrok runtime path is unsafe"
    rm -rf "$DESTINATION"
fi

command -v brew >/dev/null || fail "Homebrew is required only as a build input"
command -v cmake >/dev/null || fail "cmake build input is unavailable"
require_formula glib "$BUILD_GLIB_VERSION"
require_formula libusb "$BUILD_LIBUSB_VERSION"
require_formula gettext "$BUILD_GETTEXT_VERSION"
require_formula pcre2 "$BUILD_PCRE2_VERSION"
GLIB_OPT="$(brew --prefix glib)"
LIBUSB_OPT="$(brew --prefix libusb)"
GETTEXT_OPT="$(brew --prefix gettext)"
PCRE2_OPT="$(brew --prefix pcre2)"
GLIB_ROOT="$(cd "$GLIB_OPT" && pwd -P)"
LIBUSB_ROOT="$(cd "$LIBUSB_OPT" && pwd -P)"
GETTEXT_ROOT="$(cd "$GETTEXT_OPT" && pwd -P)"
PCRE2_ROOT="$(cd "$PCRE2_OPT" && pwd -P)"
[[ "$(hash_tree "$GLIB_ROOT" include/glib-2.0 lib/glib-2.0/include lib/pkgconfig)" == \
    "$BUILD_GLIB_TREE_SHA256" &&
   "$(hash_tree "$LIBUSB_ROOT" include/libusb-1.0 lib/pkgconfig)" == \
    "$BUILD_LIBUSB_TREE_SHA256" &&
   "$(hash_tree "$GETTEXT_ROOT" include)" == \
    "$BUILD_GETTEXT_TREE_SHA256" &&
   "$(hash_tree "$PCRE2_ROOT" include lib/pkgconfig)" == \
    "$BUILD_PCRE2_TREE_SHA256" ]] ||
    fail "native sigrok build headers or pkg-config files do not match"
for specification in \
    "$GLIB_ROOT/lib/libglib-2.0.0.dylib:$BUILD_GLIB_DYLIB_SIZE:$BUILD_GLIB_DYLIB_SHA256" \
    "$LIBUSB_ROOT/lib/libusb-1.0.0.dylib:$BUILD_LIBUSB_DYLIB_SIZE:$BUILD_LIBUSB_DYLIB_SHA256" \
    "$GETTEXT_ROOT/lib/libintl.8.dylib:$BUILD_GETTEXT_DYLIB_SIZE:$BUILD_GETTEXT_DYLIB_SHA256" \
    "$PCRE2_ROOT/lib/libpcre2-8.0.dylib:$BUILD_PCRE2_DYLIB_SIZE:$BUILD_PCRE2_DYLIB_SHA256"; do
    input="${specification%%:*}"
    remainder="${specification#*:}"
    expected_size="${remainder%%:*}"
    expected_hash="${remainder#*:}"
    [[ -f "$input" && ! -L "$input" &&
       "$(stat -f '%z' "$input")" == "$expected_size" &&
       "$(sha256_file "$input")" == "$expected_hash" ]] ||
        fail "native sigrok build library bytes do not match the lock"
done

[[ ! -L "$ROOT/.tmp" ]] || fail "repository temporary directory is unsafe"
mkdir -p "$ROOT/.tmp"
WORK="$(mktemp -d "$ROOT/.tmp/sigrok-bootstrap.XXXXXX")"
SRC="$WORK/src"
BUILD="$WORK/build"
PREFIX="$WORK/prefix"
RUNTIME="$WORK/runtime"
mkdir -p "$SRC" "$BUILD" "$PREFIX" "$RUNTIME/bin" "$RUNTIME/lib" \
    "$RUNTIME/share/sigrok-firmware" "$RUNTIME/licenses"

download "$PKGCONF_URL" "$PKGCONF_SHA256" "$WORK/pkgconf.tar.xz"
download "$LIBZIP_URL" "$LIBZIP_SHA256" "$WORK/libzip.tar.xz"
download "$LIBSIGROK_URL" "$LIBSIGROK_SHA256" "$WORK/libsigrok.tar.gz"
download "$SIGROK_CLI_URL" "$SIGROK_CLI_SHA256" "$WORK/sigrok-cli.tar.gz"
download "$FIRMWARE_DMG_URL" "$FIRMWARE_DMG_SHA256" "$WORK/sigrok.dmg"
[[ "$(stat -f '%z' "$WORK/sigrok.dmg")" == "$FIRMWARE_DMG_SIZE" ]] ||
    fail "sigrok firmware DMG size does not match"

tar -xJf "$WORK/pkgconf.tar.xz" -C "$SRC"
tar -xJf "$WORK/libzip.tar.xz" -C "$SRC"
tar -xzf "$WORK/libsigrok.tar.gz" -C "$SRC"
tar -xzf "$WORK/sigrok-cli.tar.gz" -C "$SRC"

(
    cd "$BUILD"
    mkdir pkgconf
    cd pkgconf
    "$SRC/pkgconf-$PKGCONF_VERSION/configure" \
        --prefix="$PREFIX" \
        --disable-static \
        --enable-shared
    make -j"$(sysctl -n hw.logicalcpu)"
    make install
)

cmake \
    -S "$SRC/libzip-$LIBZIP_VERSION" \
    -B "$BUILD/libzip" \
    -DCMAKE_INSTALL_PREFIX="$PREFIX" \
    -DCMAKE_OSX_ARCHITECTURES=arm64 \
    -DCMAKE_BUILD_TYPE=Release \
    -DBUILD_SHARED_LIBS=ON \
    -DBUILD_TOOLS=OFF \
    -DBUILD_REGRESS=OFF \
    -DBUILD_EXAMPLES=OFF \
    -DBUILD_DOC=OFF \
    -DBUILD_OSSFUZZ=OFF \
    -DENABLE_BZIP2=OFF \
    -DENABLE_LZMA=OFF \
    -DENABLE_ZSTD=OFF \
    -DENABLE_COMMONCRYPTO=OFF \
    -DENABLE_GNUTLS=OFF \
    -DENABLE_MBEDTLS=OFF \
    -DENABLE_OPENSSL=OFF \
    -DENABLE_WINDOWS_CRYPTO=OFF
cmake --build "$BUILD/libzip" --parallel
cmake --install "$BUILD/libzip"

export PKG_CONFIG="$PREFIX/bin/pkgconf"
export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig:$GLIB_ROOT/lib/pkgconfig:$LIBUSB_ROOT/lib/pkgconfig:$PCRE2_ROOT/lib/pkgconfig:$GETTEXT_ROOT/lib/pkgconfig"
export CC="clang -arch arm64"
mkdir "$BUILD/libsigrok"
(
    cd "$BUILD/libsigrok"
    GIT_DIR="$WORK/no-git" \
        "$SRC/libsigrok-$LIBSIGROK_VERSION/configure" \
        --prefix="$PREFIX" \
        --disable-all-drivers \
        --enable-fx2lafw \
        --disable-bindings
    make -j"$(sysctl -n hw.logicalcpu)"
    make install
)

mkdir "$BUILD/sigrok-cli"
(
    cd "$BUILD/sigrok-cli"
    CPPFLAGS="-I$PREFIX/include" \
    LDFLAGS="-L$PREFIX/lib" \
    GIT_DIR="$WORK/no-git" \
        "$SRC/sigrok-cli-$SIGROK_CLI_VERSION/configure" \
            --prefix="$PREFIX" \
            --without-libsigrokdecode
    make -j"$(sysctl -n hw.logicalcpu)"
    make install
)

/usr/bin/install -m 0755 "$PREFIX/bin/sigrok-cli" \
    "$RUNTIME/bin/sigrok-cli"
/usr/bin/install -m 0644 "$PREFIX/lib/libsigrok.4.dylib" \
    "$RUNTIME/lib/libsigrok.4.dylib"
/usr/bin/install -m 0644 "$PREFIX/lib/libzip.5.dylib" \
    "$RUNTIME/lib/libzip.5.dylib"
/usr/bin/install -m 0644 "$GLIB_ROOT/lib/libglib-2.0.0.dylib" \
    "$RUNTIME/lib/libglib-2.0.0.dylib"
/usr/bin/install -m 0644 "$GETTEXT_ROOT/lib/libintl.8.dylib" \
    "$RUNTIME/lib/libintl.8.dylib"
/usr/bin/install -m 0644 "$LIBUSB_ROOT/lib/libusb-1.0.0.dylib" \
    "$RUNTIME/lib/libusb-1.0.0.dylib"
/usr/bin/install -m 0644 "$PCRE2_ROOT/lib/libpcre2-8.0.dylib" \
    "$RUNTIME/lib/libpcre2-8.0.dylib"

/usr/bin/clang -arch arm64 \
    -I"$LIBUSB_ROOT/include/libusb-1.0" \
    "$ROOT/tools/sigrok_usb_identity.c" \
    -L"$LIBUSB_ROOT/lib" \
    -lusb-1.0 \
    -o "$RUNTIME/bin/sigrok-usb-identity"

CLI="$RUNTIME/bin/sigrok-cli"
LIBSIGROK="$RUNTIME/lib/libsigrok.4.dylib"
LIBGLIB="$RUNTIME/lib/libglib-2.0.0.dylib"
LIBINTL="$RUNTIME/lib/libintl.8.dylib"
LIBUSB="$RUNTIME/lib/libusb-1.0.0.dylib"
LIBZIP="$RUNTIME/lib/libzip.5.dylib"
LIBPCRE2="$RUNTIME/lib/libpcre2-8.0.dylib"
USB_IDENTITY="$RUNTIME/bin/sigrok-usb-identity"

patch_dependency "$CLI" "$PREFIX/lib/libsigrok.4.dylib" \
    '@executable_path/../lib/libsigrok.4.dylib'
patch_dependency "$CLI" "$GLIB_OPT/lib/libglib-2.0.0.dylib" \
    '@executable_path/../lib/libglib-2.0.0.dylib'
patch_dependency "$CLI" "$GETTEXT_OPT/lib/libintl.8.dylib" \
    '@executable_path/../lib/libintl.8.dylib'
patch_dependency "$USB_IDENTITY" \
    "$LIBUSB_OPT/lib/libusb-1.0.0.dylib" \
    '@executable_path/../lib/libusb-1.0.0.dylib'

/usr/bin/install_name_tool -id '@loader_path/libsigrok.4.dylib' "$LIBSIGROK"
patch_dependency "$LIBSIGROK" "$PREFIX/lib/libsigrok.4.dylib" \
    '@loader_path/libsigrok.4.dylib'
patch_dependency "$LIBSIGROK" "$GLIB_OPT/lib/libglib-2.0.0.dylib" \
    '@loader_path/libglib-2.0.0.dylib'
patch_dependency "$LIBSIGROK" "$GETTEXT_OPT/lib/libintl.8.dylib" \
    '@loader_path/libintl.8.dylib'
patch_dependency "$LIBSIGROK" "$LIBUSB_OPT/lib/libusb-1.0.0.dylib" \
    '@loader_path/libusb-1.0.0.dylib'
patch_dependency "$LIBSIGROK" '@rpath/libzip.5.dylib' \
    '@loader_path/libzip.5.dylib'

/usr/bin/install_name_tool -id '@loader_path/libglib-2.0.0.dylib' "$LIBGLIB"
patch_dependency "$LIBGLIB" "$GETTEXT_OPT/lib/libintl.8.dylib" \
    '@loader_path/libintl.8.dylib'
patch_dependency "$LIBGLIB" "$PCRE2_OPT/lib/libpcre2-8.0.dylib" \
    '@loader_path/libpcre2-8.0.dylib'
/usr/bin/install_name_tool -id '@loader_path/libintl.8.dylib' "$LIBINTL"
/usr/bin/install_name_tool -id '@loader_path/libusb-1.0.0.dylib' "$LIBUSB"
/usr/bin/install_name_tool -id '@loader_path/libzip.5.dylib' "$LIBZIP"
/usr/bin/install_name_tool -id '@loader_path/libpcre2-8.0.dylib' "$LIBPCRE2"

for binary in "$CLI" "$USB_IDENTITY" "$LIBSIGROK" "$LIBGLIB" "$LIBINTL" "$LIBUSB" \
    "$LIBZIP" "$LIBPCRE2"; do
    /usr/bin/codesign --force --sign - "$binary" >/dev/null
done

MOUNT="$WORK/mount"
mkdir "$MOUNT"
hdiutil attach "$WORK/sigrok.dmg" -mountpoint "$MOUNT" \
    -nobrowse -readonly -quiet
FIRMWARE_SOURCE="$MOUNT/sigrok-cli.app/Contents/share/sigrok-firmware/fx2lafw-saleae-logic.fw"
[[ -f "$FIRMWARE_SOURCE" && ! -L "$FIRMWARE_SOURCE" ]] ||
    fail "official Saleae-profile firmware is unavailable"
[[ "$(stat -f '%z' "$FIRMWARE_SOURCE")" == \
    "$FX2LAFW_SALEAE_LOGIC_SIZE" &&
   "$(sha256_file "$FIRMWARE_SOURCE")" == \
    "$FX2LAFW_SALEAE_LOGIC_SHA256" ]] ||
    fail "official Saleae-profile firmware does not match"
/usr/bin/install -m 0444 "$FIRMWARE_SOURCE" \
    "$RUNTIME/share/sigrok-firmware/fx2lafw-saleae-logic.fw"
hdiutil detach "$MOUNT" -quiet
MOUNT=""

/usr/bin/install -m 0444 "$SRC/libsigrok-$LIBSIGROK_VERSION/COPYING" \
    "$RUNTIME/licenses/libsigrok-COPYING"
/usr/bin/install -m 0444 "$SRC/sigrok-cli-$SIGROK_CLI_VERSION/COPYING" \
    "$RUNTIME/licenses/sigrok-cli-COPYING"
/usr/bin/install -m 0444 "$SRC/libzip-$LIBZIP_VERSION/LICENSE" \
    "$RUNTIME/licenses/libzip-LICENSE"
/usr/bin/install -m 0444 "$SRC/pkgconf-$PKGCONF_VERSION/COPYING" \
    "$RUNTIME/licenses/pkgconf-COPYING"
/usr/bin/install -m 0444 "$LOCK" "$RUNTIME/sigrok-cli.lock"

audit_runtime "$RUNTIME"
[[ "$(sha256_file "$RUNTIME/share/sigrok-firmware/fx2lafw-saleae-logic.fw")" == \
    "$FX2LAFW_SALEAE_LOGIC_SHA256" ]] ||
    fail "installed sigrok firmware hash changed"
"$CLI" --version | /usr/bin/grep -F "sigrok-cli $SIGROK_CLI_VERSION" \
    >/dev/null || fail "native sigrok CLI version is invalid"
"$CLI" --list-supported | /usr/bin/grep -F "fx2lafw" >/dev/null ||
    fail "native sigrok CLI lacks fx2lafw"

(
    cd "$RUNTIME"
    /usr/bin/find . -type f ! -name MANIFEST.sha256 -print |
        LC_ALL=C /usr/bin/sort |
        while IFS= read -r path; do
            /usr/bin/shasum -a 256 "$path"
        done >MANIFEST.sha256
)
chmod -R go-w "$RUNTIME"
mkdir -p "$(dirname "$DESTINATION")"
[[ ! -e "$DESTINATION" && ! -L "$DESTINATION" ]] ||
    fail "sigrok destination appeared during bootstrap"
mv "$RUNTIME" "$DESTINATION"
printf 'Installed native arm64 sigrok CLI: %s\n' "$DESTINATION"
"$DESTINATION/bin/sigrok-cli" --version | /usr/bin/head -1
