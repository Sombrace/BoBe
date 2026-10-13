#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ESP_ROOT="${BOBE_ESP_SDK_ROOT:-$ROOT/esp32-sdk}"
RUST="$ROOT/BoBeService/src/body/protocol.rs"
RUST_BODY_CONFIG="$ROOT/BoBeService/src/config/body.rs"
RUST_GATEWAY="$ROOT/BoBeService/src/body/gateway.rs"
RUST_UPDATE="$ROOT/BoBeService/src/body/update.rs"
SWIFT="$ROOT/BoBeMacUI/BoBe/BodyLink/BodyAdapterProtocol.swift"
SWIFT_SETTINGS="$ROOT/BoBeMacUI/BoBe/DTOs/SettingsTypes.swift"
SWIFT_ADAPTER="$ROOT/BoBeMacUI/BoBe/BodyLink/BodySpeechAdapter.swift"
C_HEADER="$ESP_ROOT/components/bobe_protocol/include/bobe_protocol.h"
C_FRAME="$ESP_ROOT/components/bobe_protocol/bobe_frame.c"
C_TYPES="$ESP_ROOT/components/bobe_protocol/include/bobe_types.h"
C_TRANSPORT="$ESP_ROOT/components/bobe_transport/bobe_transport.c"
C_CONNECTIVITY="$ESP_ROOT/components/bobe_connectivity/bobe_connectivity.c"
C_CONTROL="$ESP_ROOT/components/bobe_protocol/bobe_control.c"
C_UPDATE_HEADER="$ESP_ROOT/components/bobe_protocol/include/bobe_update_manifest.h"
C_UPDATE_SOURCE="$ESP_ROOT/components/bobe_protocol/bobe_update_manifest.c"
C_DEVICE_CONFIG="$ESP_ROOT/examples/wb12/components/bobe_device_config/include/bobe_device_config.h"
PYTHON_UPDATE="$ESP_ROOT/examples/wb12/tools/create_update_offer.py"
TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/bobe-bodylink-contract.XXXXXX")"
trap 'rm -rf "$TEMP_DIR"' EXIT

for file in \
    "$RUST" \
    "$RUST_BODY_CONFIG" \
    "$RUST_GATEWAY" \
    "$RUST_UPDATE" \
    "$SWIFT" \
    "$SWIFT_SETTINGS" \
    "$SWIFT_ADAPTER" \
    "$C_HEADER" \
    "$C_FRAME" \
    "$C_TYPES" \
    "$C_TRANSPORT" \
    "$C_CONNECTIVITY" \
    "$C_CONTROL" \
    "$C_UPDATE_HEADER" \
    "$C_UPDATE_SOURCE" \
    "$C_DEVICE_CONFIG" \
    "$PYTHON_UPDATE"; do
    if [[ ! -f "$file" ]]; then
        echo "BodyLink contract source missing: $file" >&2
        exit 1
    fi
done

fail=0
expect_match() {
    local name="$1"
    local left="$2"
    local right="$3"
    if [[ "$left" != "$right" ]]; then
        echo "BodyLink drift: $name — '$left' != '$right'" >&2
        fail=1
    fi
}

rust_number() {
    grep -E "const $1: [^=]+=" "$RUST" | sed -E 's/.*= ([0-9_]+);.*/\1/' | tr -d '_'
}
rust_hex() {
    grep -E "const $1: [^=]+=" "$RUST" | sed -E 's/.*= (0x[0-9A-Fa-f]+);.*/\1/' | tr 'A-F' 'a-f'
}
c_number() {
    grep -E "#define $1 " "$C_HEADER" | sed -E 's/.* ([0-9]+)U.*/\1/'
}
c_hex() {
    grep -E "#define $1 " "$C_HEADER" | sed -E 's/.* (0x[0-9A-Fa-f]+)U.*/\1/' | tr 'A-F' 'a-f'
}
c_types_number() {
    grep -E "#define $1 " "$C_TYPES" | sed -E 's/.* ([0-9]+)U.*/\1/'
}
rust_string() {
    grep -E "const $1: &str =" "$RUST" | sed -E 's/.*"([^"]+)".*/\1/'
}
c_string() {
    grep -E "#define $1 " "$C_HEADER" | sed -E 's/.*"([^"]+)".*/\1/'
}

expect_match "protocol major" "$(rust_number PROTOCOL_MAJOR)" "$(c_number BOBE_PROTOCOL_MAJOR)"
expect_match "protocol minor" "$(rust_number PROTOCOL_MINOR)" "$(c_number BOBE_PROTOCOL_MINOR)"
expect_match "minimum supported body minor" "$(rust_number MIN_SUPPORTED_PROTOCOL_MINOR)" "$(c_number BOBE_PROTOCOL_MIN_MINOR)"
swift_adapter_minor=$(sed -n 's/.*encode(\([0-9][0-9]*\), forKey: .protocolMinor).*/\1/p' "$SWIFT")
expect_match "speech adapter minor" "$(rust_number ADAPTER_PROTOCOL_MINOR)" "$swift_adapter_minor"
expect_match \
    "maximum control message bytes" \
    "$(rust_number MAX_CONTROL_BYTES)" \
    "$(c_types_number BOBE_CONTROL_MAX_BYTES)"
expect_match "BodyLink media header" "$(rust_number MEDIA_HEADER_LEN)" "$(c_number BOBE_MEDIA_HEADER_BYTES)"
expect_match \
    "maximum playback credit" \
    "$(rust_number MAX_PLAYBACK_CREDIT_MS)" \
    "$(c_number BOBE_MAX_PLAYBACK_CREDIT_MS)"
expect_match \
    "maximum lease lifetime" \
    "$(rust_number MAX_LEASE_LIFETIME_MS)" \
    "$(c_number BOBE_MAX_LEASE_LIFETIME_MS)"
command -v cc >/dev/null
cat >"$TEMP_DIR/audio_sizes.c" <<'EOF'
#include "bobe_types.h"
#include <stdio.h>
int main(void) {
    printf(
        "%zu %zu\n",
        sizeof(int16_t) * (size_t)BOBE_MIC_FRAME_SAMPLES,
        sizeof(int16_t) * (size_t)BOBE_SPEAKER_FRAME_SAMPLES
    );
    return 0;
}
EOF
cc -std=c11 -Wall -Wextra -Werror \
    -I"$ESP_ROOT/components/bobe_protocol/include" \
    "$TEMP_DIR/audio_sizes.c" -o "$TEMP_DIR/audio_sizes"
read -r c_mic_pcm_bytes c_speaker_pcm_bytes < <("$TEMP_DIR/audio_sizes")
expect_match "microphone PCM bytes" "$(rust_number MIC_PCM_BYTES)" "$c_mic_pcm_bytes"
expect_match "speaker PCM bytes" "$(rust_number SPEAKER_PCM_BYTES)" "$c_speaker_pcm_bytes"

swift_body_header=$(awk '/struct BodyLinkMicrophoneFrame/,/^}/' "$SWIFT" \
    | grep -E 'static let headerLength = ' | sed -E 's/.*= ([0-9]+).*/\1/')
swift_body_payload=$(awk '/struct BodyLinkMicrophoneFrame/,/^}/' "$SWIFT" \
    | grep -E 'static let payloadLength = ' | sed -E 's/.*= ([0-9]+).*/\1/')
swift_discontinuity=$(awk '/struct BodyLinkMicrophoneFrame/,/^}/' "$SWIFT" \
    | grep -E 'static let discontinuityFlag:' | sed -E 's/.*= (0x[0-9A-Fa-f]+).*/\1/' \
    | tr 'A-F' 'a-f')
rust_discontinuity=$(rust_hex MEDIA_FLAG_DISCONTINUITY)
c_discontinuity=$(c_hex BOBE_MEDIA_FLAG_DISCONTINUITY)
expect_match "Swift BodyLink media header" "$(rust_number MEDIA_HEADER_LEN)" "$swift_body_header"
expect_match "Swift microphone PCM bytes" "$(rust_number MIC_PCM_BYTES)" "$swift_body_payload"
expect_match "microphone discontinuity flag Rust/Swift" "$rust_discontinuity" "$swift_discontinuity"
expect_match "microphone discontinuity flag Rust/firmware" "$rust_discontinuity" "$c_discontinuity"
grep -Fq 'flags & !MEDIA_FLAG_DISCONTINUITY' "$RUST" || {
    echo "BodyLink drift: Rust media parser does not reject reserved flags" >&2
    fail=1
}
grep -Fq 'flags & ~self.discontinuityFlag == 0' "$SWIFT" || {
    echo "BodyLink drift: Swift media parser does not reject reserved flags" >&2
    fail=1
}
grep -Fq 'header->flags & ~BOBE_MEDIA_FLAG_DISCONTINUITY' "$C_FRAME" || {
    echo "BodyLink drift: firmware media parser does not reject reserved flags" >&2
    fail=1
}

swift_format_field() {
    local format="$1"
    local field="$2"
    awk "/static let $format = Self\\(/,/^    \\)/" "$SWIFT" \
        | grep -E "^[[:space:]]*$field:" \
        | sed -E 's/.*: "?([^",]+)"?,?/\1/'
}
swift_format_number() {
    swift_format_field "$1" "$2" | tr -d '_'
}

expect_match \
    "capture codec" \
    "$(rust_string CAPTURE_CODEC)" \
    "$(swift_format_field capture codec)"
expect_match \
    "capture sample rate Rust/Swift" \
    "$(rust_number CAPTURE_SAMPLE_RATE_HZ)" \
    "$(swift_format_number capture sampleRateHz)"
expect_match \
    "capture sample rate Rust/firmware" \
    "$(rust_number CAPTURE_SAMPLE_RATE_HZ)" \
    "$(c_types_number BOBE_MIC_SAMPLE_RATE_HZ)"
expect_match \
    "capture channels" \
    "$(rust_number CAPTURE_CHANNELS)" \
    "$(swift_format_number capture channels)"
expect_match \
    "audio frame duration Rust/Swift" \
    "$(rust_number AUDIO_FRAME_DURATION_MS)" \
    "$(swift_format_number capture frameDurationMs)"
expect_match \
    "audio frame duration Rust/firmware" \
    "$(rust_number AUDIO_FRAME_DURATION_MS)" \
    "$(c_types_number BOBE_AUDIO_FRAME_DURATION_MS)"
expect_match \
    "speaker sample rate Rust/firmware" \
    "$(rust_number SPEAKER_SAMPLE_RATE_HZ)" \
    "$(c_types_number BOBE_SPEAKER_SAMPLE_RATE_HZ)"

swift_adapter_header=$(awk '/struct BodyAdapterMediaHeader/,/^}/' "$SWIFT" \
    | grep -E 'static let length = ' | sed -E 's/.*= ([0-9]+).*/\1/')
expect_match "adapter media header" "$(rust_number ADAPTER_MEDIA_HEADER_LEN)" "$swift_adapter_header"

rust_body_protocol=$(grep -E 'BODY_SUBPROTOCOL_V1: &str =' "$RUST" | sed -E 's/.*"([^"]+)".*/\1/')
c_body_protocol=$(grep -E '\.subprotocol = ' "$C_TRANSPORT" | sed -E 's/.*"([^"]+)".*/\1/')
expect_match "body WS subprotocol" "$rust_body_protocol" "$c_body_protocol"
expect_match \
    "BodyLink mDNS auth" \
    "$(rust_string BODY_MDNS_AUTH)" \
    "$(c_string BOBE_BODY_MDNS_AUTH)"

if ! grep -Fq 'txt_matches(item, "auth", BOBE_BODY_MDNS_AUTH)' "$C_CONNECTIVITY"; then
    echo "BodyLink drift: ESP discovery does not enforce BOBE_BODY_MDNS_AUTH" >&2
    fail=1
fi

rust_adapter_protocol=$(grep -E 'ADAPTER_SUBPROTOCOL_V1: &str =' "$RUST" | sed -E 's/.*"([^"]+)".*/\1/')
swift_adapter_protocol=$(grep -E 'Sec-WebSocket-Protocol' "$SWIFT_ADAPTER" | sed -E 's/.*"([^"]+)", forHTTP.*/\1/')
expect_match "adapter WS subprotocol" "$rust_adapter_protocol" "$swift_adapter_protocol"

python3 - "$RUST_BODY_CONFIG" "$SWIFT_SETTINGS" "$RUST_GATEWAY" "$C_CONTROL" <<'PY'
import re
import sys
from pathlib import Path

rust_body_config, swift_settings, rust_gateway, c_control = map(
    lambda value: Path(value).read_text(), sys.argv[1:]
)

def enum_body(source, name):
    match = re.search(rf"enum\s+{name}\b[^{{]*\{{(.*?)^}}", source, re.MULTILINE | re.DOTALL)
    if match is None:
        raise SystemExit(f"BodyLink contract parser could not find {name}")
    return match.group(1)

def camel_to_kebab(value):
    return re.sub(r"(?<!^)(?=[A-Z])", "-", value).lower()

rust_themes = {
    camel_to_kebab(value)
    for value in re.findall(r"^\s*(?:#\[[^]]+\]\s*)?(\w+)\s*,", enum_body(rust_body_config, "BodyTheme"), re.MULTILINE)
}
swift_themes = set(
    re.findall(r'^\s*case\s+\w+\s*=\s*"([^"]+)"', enum_body(swift_settings, "PhysicalBodyTheme"), re.MULTILINE)
)

def c_name_array(function):
    match = re.search(
        rf"{function}\([^)]*\)\s*\{{.*?static const char \*const names\[\] = \{{(.*?)\}};",
        c_control,
        re.DOTALL,
    )
    if match is None:
        raise SystemExit(f"BodyLink contract parser could not find {function} names")
    return set(re.findall(r'"([a-z0-9_-]+)"', match.group(1)))

firmware_themes = c_name_array("bobe_face_theme_name")
for label, values in (("Swift", swift_themes), ("firmware", firmware_themes)):
    if values != rust_themes:
        raise SystemExit(
            f"BodyLink drift: physical body themes Rust/{label} — "
            f"missing={sorted(rust_themes - values)}, extra={sorted(values - rust_themes)}"
        )

firmware_expressions = c_name_array("bobe_face_expression_name")
daemon_expressions = set(
    re.findall(r'send_face\(\s*[^,]+,\s*"([a-z_]+)"', rust_gateway, re.DOTALL)
)
daemon_expressions.update(re.findall(r'"expression"\s*:\s*"([a-z_]+)"', rust_gateway))
if not daemon_expressions:
    raise SystemExit("BodyLink contract parser found no daemon face expressions")
unsupported = daemon_expressions - firmware_expressions
if unsupported:
    raise SystemExit(
        f"BodyLink drift: daemon emits unsupported face expressions: {sorted(unsupported)}"
    )
PY

python3 - "$RUST_UPDATE" "$C_UPDATE_HEADER" "$C_UPDATE_SOURCE" "$PYTHON_UPDATE" "$(c_types_number BOBE_CONTROL_MAX_BYTES)" "$C_DEVICE_CONFIG" <<'PY'
import ast
import re
import sys
from pathlib import Path

rust, c_header, c_source, python_source = (Path(value).read_text() for value in sys.argv[1:5])
firmware_control_bytes = int(sys.argv[5])
c_header += "\n" + Path(sys.argv[6]).read_text()

def rust_string(name):
    match = re.search(rf'const {name}: &str = "([^"]+)";', rust)
    if match is None:
        raise SystemExit(f"BodyLink OTA parser could not find Rust {name}")
    return match.group(1)

def rust_number(name):
    match = re.search(rf"const {name}: [^=]+ = ([^;]+);", rust)
    if match is None:
        raise SystemExit(f"BodyLink OTA parser could not find Rust {name}")
    expression = match.group(1).replace("_", "")
    if re.fullmatch(r"[0-9]+(?: *\* *[0-9]+)*", expression) is None:
        raise SystemExit(f"BodyLink OTA parser cannot evaluate Rust {name}: {expression}")
    return eval(expression, {"__builtins__": {}}, {})

def c_string(name):
    match = re.search(rf'^#define {name} "([^"]+)"', c_header, re.MULTILINE)
    if match is None:
        raise SystemExit(f"BodyLink OTA parser could not find firmware {name}")
    return match.group(1)

def c_number(name):
    match = re.search(rf"^#define {name} (.+)$", c_header, re.MULTILINE)
    if match is None:
        raise SystemExit(f"BodyLink OTA parser could not find firmware {name}")
    value = match.group(1).strip()
    value = re.sub(r"^(?:U?INT(?:32|64)_C)\((.+)\)$", r"\1", value)
    value = re.sub(r"[uUlL]+$", "", value)
    return int(value, 0)

def python_assignments():
    tree = ast.parse(python_source)
    values = {}
    for node in tree.body:
        if isinstance(node, ast.Assign) and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name):
            try:
                values[node.targets[0].id] = ast.literal_eval(node.value)
            except (ValueError, TypeError):
                pass
    return values

python_values = python_assignments()
comparisons = [
    ("maximum offer bytes", rust_number("MAX_OFFER_BYTES"), firmware_control_bytes, python_values.get("MAX_OFFER_BYTES")),
    ("WB-12 device model", rust_string("DEVICE_MODEL"), c_string("BOBE_DEVICE_MODEL"), python_values.get("MODEL")),
    ("schema version", rust_number("SCHEMA_VERSION"), c_number("BOBE_UPDATE_SCHEMA_VERSION"), python_values.get("SCHEMA_VERSION")),
    ("maximum image bytes", rust_number("MAX_IMAGE_BYTES"), c_number("BOBE_UPDATE_IMAGE_SIZE_MAX"), python_values.get("MAX_IMAGE")),
    ("maximum safe integer", rust_number("MAX_SAFE_INTEGER"), c_number("BOBE_UPDATE_SAFE_INTEGER_MAX"), python_values.get("MAX_SAFE_INTEGER")),
    ("maximum version length", rust_number("MAX_VERSION_LEN"), c_number("BOBE_UPDATE_VERSION_MAX"), python_values.get("MAX_VERSION_LEN")),
    ("maximum URL length", rust_number("MAX_URL_LEN"), c_number("BOBE_UPDATE_URL_MAX"), python_values.get("MAX_URL_LEN")),
    ("maximum key ID length", rust_number("MAX_KEY_ID_LEN"), c_number("BOBE_UPDATE_KEY_ID_MAX"), python_values.get("MAX_KEY_ID_LEN")),
    ("maximum manifest bytes", rust_number("MAX_MANIFEST_BYTES"), c_number("BOBE_UPDATE_MANIFEST_MAX_BYTES"), python_values.get("MAX_MANIFEST_BYTES")),
]
if python_values.get("MAX_BUILD_NUMBER") != 0xFFFFFFFF:
    raise SystemExit(
        f"BodyLink OTA drift: Python build number maximum — "
        f"{python_values.get('MAX_BUILD_NUMBER')!r} != 4294967295"
    )
for label, rust_value, firmware_value, python_value in comparisons:
    if not (rust_value == firmware_value == python_value):
        raise SystemExit(
            f"BodyLink OTA drift: {label} — Rust={rust_value!r}, "
            f"firmware={firmware_value!r}, Python={python_value!r}"
        )

def rust_array(name):
    match = re.search(rf"const {name}: \[&str; \d+\] = \[(.*?)\];", rust, re.DOTALL)
    if match is None:
        raise SystemExit(f"BodyLink OTA parser could not find Rust {name}")
    return set(re.findall(r'"([a-z0-9_]+)"', match.group(1)))

python_tree = ast.parse(python_source)
manifest_fields = set()
envelope_fields = set()
for node in ast.walk(python_tree):
    if isinstance(node, ast.Assign):
        for target in node.targets:
            if isinstance(target, ast.Name) and target.id in {"manifest", "offer"} and isinstance(node.value, ast.Dict):
                fields = {
                    key.value for key in node.value.keys
                    if isinstance(key, ast.Constant) and isinstance(key.value, str)
                }
                if target.id == "manifest":
                    manifest_fields |= fields
                else:
                    envelope_fields |= fields

firmware_field_table = re.search(
    r"field_names\[BOBE_FIELD_COUNT\]\s*=\s*\{(.*?)\};",
    c_source,
    re.DOTALL,
)
if firmware_field_table is None:
    raise SystemExit("BodyLink OTA parser could not find firmware manifest field names")
firmware_manifest_fields = set(re.findall(r'"([a-z0-9_]+)"', firmware_field_table.group(1)))
for label, expected, actual in (
    ("manifest fields Rust/firmware", rust_array("MANIFEST_FIELDS"), firmware_manifest_fields),
    ("manifest fields Rust/Python", rust_array("MANIFEST_FIELDS"), manifest_fields),
    ("envelope fields Rust/Python", rust_array("ENVELOPE_FIELDS"), envelope_fields),
):
    if expected != actual:
        raise SystemExit(
            f"BodyLink OTA drift: {label} — missing={sorted(expected - actual)}, "
            f"extra={sorted(actual - expected)}"
        )
PY

if [[ "$fail" -ne 0 ]]; then
    exit 1
fi
echo "BodyLink Rust/Swift/ESP contract constants match."
