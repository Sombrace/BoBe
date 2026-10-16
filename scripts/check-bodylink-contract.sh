#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PYTHONDONTWRITEBYTECODE=1 exec python3 - "$ROOT" "${BOBE_ESP_SDK_ROOT:-$ROOT/esp32-sdk}" <<'PY'
import ast
import re
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(sys.argv[1]) / "scripts/lib"))
from source_contracts import Contract

contract = Contract(sys.argv[1], "BodyLink")
esp = Path(sys.argv[2]).resolve()
RUST = "BoBeService/src/body/protocol.rs"
RUST_BODY_CONFIG = "BoBeService/src/config/body.rs"
RUST_GATEWAY = "BoBeService/src/body/gateway.rs"
RUST_HANDLERS = "BoBeService/src/body/handlers.rs"
RUST_UPDATE = "BoBeService/src/body/update.rs"
SWIFT_PROTOCOL = "BoBeMacUI/BoBe/BodyLink/BodyAdapterProtocol.swift"
SWIFT_ADAPTER = "BoBeMacUI/BoBe/BodyLink/BodySpeechAdapter.swift"
C_INCLUDE = esp / "components/bobe_protocol/include"
C_HEADER = C_INCLUDE / "bobe_protocol.h"
C_TYPES = C_INCLUDE / "bobe_types.h"
C_UPDATE_HEADER = C_INCLUDE / "bobe_update_manifest.h"
C_FRAME = esp / "components/bobe_protocol/bobe_frame.c"
C_CONTROL = esp / "components/bobe_protocol/bobe_control.c"
C_INTERACTION = esp / "components/bobe_protocol/bobe_interaction.c"
C_UPDATE_SOURCE = esp / "components/bobe_protocol/bobe_update_manifest.c"
C_TRANSPORT = esp / "components/bobe_transport/bobe_transport.c"
C_CONNECTIVITY = esp / "components/bobe_connectivity/bobe_connectivity.c"
C_DEVICE_CONFIG = esp / "examples/wb12/components/bobe_device_config/include/bobe_device_config.h"
PYTHON_UPDATE = esp / "examples/wb12/tools/create_update_offer.py"
PYTHON_HIL = esp / "examples/wb12/bobe_hil/bodylink.py"
MESSAGE = r"[a-z][a-z0-9_]*(?:\.[a-z0-9_]+)*"


def rust(name, path=RUST):
    return contract.integer(f"Rust {name}", contract.rust_const(path, name))


def rust_text(name, path=RUST):
    return contract.string(f"Rust {name}", contract.rust_const(path, name))


def firmware(name, path=C_HEADER):
    return contract.integer(f"firmware {name}", contract.c_define(path, name))


def firmware_text(name, path=C_HEADER):
    return contract.string(f"firmware {name}", contract.c_define(path, name))


def swift(owner, name):
    return contract.integer(f"Swift {owner}.{name}", contract.swift_static(owner, name))


def firmware_audio_sizes():
    compiler = shutil.which("cc")
    if compiler is None:
        contract.fail("a C compiler (cc) is required to evaluate the firmware audio frame sizes")
    contract.read(C_TYPES)
    build = contract.root / "build"
    build.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="bodylink-contract-", dir=build) as work:
        source = Path(work) / "audio_sizes.c"
        binary = Path(work) / "audio_sizes"
        source.write_text(
            '#include "bobe_types.h"\n'
            "#include <stdio.h>\n"
            "int main(void) {\n"
            '    printf("%zu %zu %zu %zu\\n",\n'
            "        (size_t)BOBE_MIC_FRAME_SAMPLES,\n"
            "        (size_t)BOBE_SPEAKER_FRAME_SAMPLES,\n"
            "        sizeof(int16_t) * (size_t)BOBE_MIC_FRAME_SAMPLES,\n"
            "        sizeof(int16_t) * (size_t)BOBE_SPEAKER_FRAME_SAMPLES);\n"
            "    return 0;\n"
            "}\n"
        )
        try:
            subprocess.run(
                [compiler, "-std=c11", "-Wall", "-Wextra", "-Werror", f"-I{C_INCLUDE}", str(source), "-o", str(binary)],
                check=True,
            )
            output = subprocess.run([str(binary)], check=True, capture_output=True, text=True).stdout.split()
        except subprocess.CalledProcessError as error:
            contract.fail(f"could not evaluate the firmware audio frame sizes: {error}")
    if len(output) != 4:
        contract.fail(f"unexpected firmware audio frame sizes: {output}")
    return [int(value) for value in output]


mic_samples, speaker_samples, mic_bytes, speaker_bytes = firmware_audio_sizes()
swift_protocol = contract.read(SWIFT_PROTOCOL)
firmware_subprotocol = contract.find(
    "firmware WebSocket subprotocol",
    r'\.subprotocol\s*=\s*"([^"]+)"',
    contract.read(C_TRANSPORT),
)

contract.expect("protocol major", rust("PROTOCOL_MAJOR"), firmware("BOBE_PROTOCOL_MAJOR"))
contract.expect("protocol minor", rust("PROTOCOL_MINOR"), firmware("BOBE_PROTOCOL_MINOR"))
contract.expect(
    "minimum supported body minor",
    rust("MIN_SUPPORTED_PROTOCOL_MINOR"),
    firmware("BOBE_PROTOCOL_MIN_MINOR"),
)
contract.expect(
    "speech adapter minor",
    rust("ADAPTER_PROTOCOL_MINOR"),
    contract.integer(
        "Swift adapter protocol minor",
        contract.find("Swift adapter protocol minor", r"\bencode\(([0-9]+), forKey: \.protocolMinor\)", swift_protocol),
    ),
)
contract.expect("maximum control message bytes", rust("MAX_CONTROL_BYTES"), firmware("BOBE_CONTROL_MAX_BYTES", C_TYPES))
contract.expect(
    "BodyLink media header",
    rust("MEDIA_HEADER_LEN"),
    firmware("BOBE_MEDIA_HEADER_BYTES"),
    swift("BodyLinkMicrophoneFrame", "headerLength"),
)
contract.expect("maximum playback credit", rust("MAX_PLAYBACK_CREDIT_MS"), firmware("BOBE_MAX_PLAYBACK_CREDIT_MS"))
contract.expect("maximum lease lifetime", rust("MAX_LEASE_LIFETIME_MS"), firmware("BOBE_MAX_LEASE_LIFETIME_MS"))
contract.expect(
    "microphone PCM bytes",
    rust("MIC_PCM_BYTES"),
    mic_bytes,
    swift("BodyLinkMicrophoneFrame", "payloadLength"),
)
contract.expect("speaker PCM bytes", rust("SPEAKER_PCM_BYTES"), speaker_bytes)
contract.expect(
    "microphone discontinuity flag",
    rust("MEDIA_FLAG_DISCONTINUITY"),
    firmware("BOBE_MEDIA_FLAG_DISCONTINUITY"),
    swift("BodyLinkMicrophoneFrame", "discontinuityFlag"),
)
contract.require(
    "Rust media parser does not reject reserved flags",
    "flags & !MEDIA_FLAG_DISCONTINUITY",
    contract.read(RUST),
)
contract.require(
    "Swift media parser does not reject reserved flags",
    "flags & ~self.discontinuityFlag == 0",
    swift_protocol,
)
contract.require(
    "firmware media parser does not reject reserved flags",
    "header->flags & ~BOBE_MEDIA_FLAG_DISCONTINUITY",
    contract.read(C_FRAME),
)

capture_format = contract.block(
    "Swift BodyAdapterAudioFormat.capture",
    contract.swift_type("BodyAdapterAudioFormat"),
    r"\bstatic let capture = Self\(",
)


def capture_field(name):
    return contract.find(f"Swift capture format {name}", rf"^\s*{name}:\s*(.+?),?\s*$", capture_format)


contract.expect(
    "capture codec",
    rust_text("CAPTURE_CODEC"),
    contract.string("Swift capture codec", capture_field("codec")),
)
contract.expect(
    "capture sample rate",
    rust("CAPTURE_SAMPLE_RATE_HZ"),
    contract.integer("Swift capture sample rate", capture_field("sampleRateHz")),
    firmware("BOBE_MIC_SAMPLE_RATE_HZ", C_TYPES),
)
contract.expect(
    "capture channels",
    rust("CAPTURE_CHANNELS"),
    contract.integer("Swift capture channels", capture_field("channels")),
)
contract.expect(
    "audio frame duration",
    rust("AUDIO_FRAME_DURATION_MS"),
    contract.integer("Swift capture frame duration", capture_field("frameDurationMs")),
    firmware("BOBE_AUDIO_FRAME_DURATION_MS", C_TYPES),
)
contract.expect(
    "speaker sample rate",
    rust("SPEAKER_SAMPLE_RATE_HZ"),
    firmware("BOBE_SPEAKER_SAMPLE_RATE_HZ", C_TYPES),
)
contract.expect("adapter media header", rust("ADAPTER_MEDIA_HEADER_LEN"), swift("BodyAdapterMediaHeader", "length"))
contract.expect("body WS subprotocol", rust_text("BODY_SUBPROTOCOL_V1"), firmware_subprotocol)
contract.expect("BodyLink mDNS auth", rust_text("BODY_MDNS_AUTH"), firmware_text("BOBE_BODY_MDNS_AUTH"))
contract.require(
    "ESP discovery does not enforce BOBE_BODY_MDNS_AUTH",
    'txt_matches(item, "auth", BOBE_BODY_MDNS_AUTH)',
    contract.read(C_CONNECTIVITY),
)
contract.expect(
    "adapter WS subprotocol",
    rust_text("ADAPTER_SUBPROTOCOL_V1"),
    contract.find(
        "Swift adapter WebSocket subprotocol",
        r'setValue\("([^"]+)", forHTTPHeaderField: "Sec-WebSocket-Protocol"\)',
        contract.read(SWIFT_ADAPTER),
    ),
)

control = contract.read(C_CONTROL)


def firmware_names(function):
    body = contract.block(f"firmware {function}", control, rf"\b{function}\([^)]*\)\s*\{{")
    names = contract.find(
        f"firmware {function} names",
        r"static const char \*const names\[\] = \{(.*?)\};",
        body,
        re.DOTALL,
    )
    return set(contract.findall(f"firmware {function} name strings", r'"([a-z0-9_-]+)"', names))


rust_themes = {theme.replace("_", "-") for theme in contract.rust_enum(RUST_BODY_CONFIG, "BodyTheme")}
contract.expect_set("physical body themes", rust_themes, contract.swift_enum("PhysicalBodyTheme"))
contract.expect_set("physical body themes", rust_themes, firmware_names("bobe_face_theme_name"), "Rust", "firmware")

gateway = contract.read(RUST_GATEWAY)
daemon_expressions = set(re.findall(r'send_face\(\s*[^,]+,\s*"([a-z_]+)"', gateway, re.DOTALL))
daemon_expressions.update(re.findall(r'"expression"\s*:\s*"([a-z_]+)"', gateway))
contract.expect_subset(
    "face expressions",
    daemon_expressions,
    firmware_names("bobe_face_expression_name"),
    "the daemon",
    "firmware",
)

update_constants = contract.python_constants(PYTHON_UPDATE)


def update_tool(name):
    return contract.python_constant(update_constants, PYTHON_UPDATE, name)


for label, rust_value, firmware_value, python_name in (
    ("maximum offer bytes", rust("MAX_OFFER_BYTES", RUST_UPDATE), firmware("BOBE_CONTROL_MAX_BYTES", C_TYPES), "MAX_OFFER_BYTES"),
    ("WB-12 device model", rust_text("DEVICE_MODEL", RUST_UPDATE), firmware_text("BOBE_DEVICE_MODEL", C_DEVICE_CONFIG), "MODEL"),
    ("schema version", rust("SCHEMA_VERSION", RUST_UPDATE), firmware("BOBE_UPDATE_SCHEMA_VERSION", C_UPDATE_HEADER), "SCHEMA_VERSION"),
    ("maximum image bytes", rust("MAX_IMAGE_BYTES", RUST_UPDATE), firmware("BOBE_UPDATE_IMAGE_SIZE_MAX", C_DEVICE_CONFIG), "MAX_IMAGE"),
    ("maximum safe integer", rust("MAX_SAFE_INTEGER", RUST_UPDATE), firmware("BOBE_UPDATE_SAFE_INTEGER_MAX", C_UPDATE_HEADER), "MAX_SAFE_INTEGER"),
    ("maximum version length", rust("MAX_VERSION_LEN", RUST_UPDATE), firmware("BOBE_UPDATE_VERSION_MAX", C_UPDATE_HEADER), "MAX_VERSION_LEN"),
    ("maximum URL length", rust("MAX_URL_LEN", RUST_UPDATE), firmware("BOBE_UPDATE_URL_MAX", C_UPDATE_HEADER), "MAX_URL_LEN"),
    ("maximum key ID length", rust("MAX_KEY_ID_LEN", RUST_UPDATE), firmware("BOBE_UPDATE_KEY_ID_MAX", C_UPDATE_HEADER), "MAX_KEY_ID_LEN"),
    ("maximum manifest bytes", rust("MAX_MANIFEST_BYTES", RUST_UPDATE), firmware("BOBE_UPDATE_MANIFEST_MAX_BYTES", C_UPDATE_HEADER), "MAX_MANIFEST_BYTES"),
):
    contract.expect(f"OTA {label}", rust_value, firmware_value, update_tool(python_name))
contract.expect("OTA Python build number maximum", update_tool("MAX_BUILD_NUMBER"), 0xFFFFFFFF)


def rust_fields(name):
    return set(contract.findall(f"Rust {name} entries", r'"([a-z0-9_]+)"', contract.rust_const(RUST_UPDATE, name)))


manifest_fields = set()
envelope_fields = set()
for node in ast.walk(ast.parse(contract.read(PYTHON_UPDATE))):
    if isinstance(node, ast.Assign) and isinstance(node.value, ast.Dict):
        keys = {key.value for key in node.value.keys if isinstance(key, ast.Constant) and isinstance(key.value, str)}
        for target in node.targets:
            if isinstance(target, ast.Name) and target.id == "manifest":
                manifest_fields |= keys
            elif isinstance(target, ast.Name) and target.id == "offer":
                envelope_fields |= keys
firmware_fields = set(
    contract.findall(
        "firmware manifest field names",
        r'"([a-z0-9_]+)"',
        contract.find(
            "firmware manifest field table",
            r"field_names\[BOBE_FIELD_COUNT\]\s*=\s*\{(.*?)\};",
            contract.read(C_UPDATE_SOURCE),
            re.DOTALL,
        ),
    )
)
contract.expect_set("OTA manifest fields", rust_fields("MANIFEST_FIELDS"), firmware_fields, "Rust", "firmware")
contract.expect_set("OTA manifest fields", rust_fields("MANIFEST_FIELDS"), manifest_fields, "Rust", "Python")
contract.expect_set("OTA envelope fields", rust_fields("ENVELOPE_FIELDS"), envelope_fields, "Rust", "Python")

hil_constants = contract.python_constants(PYTHON_HIL)


def hil(name):
    return contract.python_constant(hil_constants, PYTHON_HIL, name)


media_header = hil("MEDIA_HEADER")
if not isinstance(media_header, struct.Struct):
    contract.fail(f"{PYTHON_HIL} MEDIA_HEADER must be a struct.Struct literal")
contract.expect("HIL protocol major", hil("PROTOCOL_MAJOR"), rust("PROTOCOL_MAJOR"), firmware("BOBE_PROTOCOL_MAJOR"))
hil_minor = hil("PROTOCOL_MINOR")
for side, minimum, maximum in (
    ("Rust", rust("MIN_SUPPORTED_PROTOCOL_MINOR"), rust("PROTOCOL_MINOR")),
    ("firmware", firmware("BOBE_PROTOCOL_MIN_MINOR"), firmware("BOBE_PROTOCOL_MINOR")),
):
    if not isinstance(hil_minor, int) or not minimum <= hil_minor <= maximum:
        contract.drift(f"HIL protocol minor — {hil_minor!r} is outside the {side} range {minimum}..{maximum}")
contract.expect("HIL subprotocol", hil("SUBPROTOCOL"), rust_text("BODY_SUBPROTOCOL_V1"), firmware_subprotocol)
contract.expect("HIL media header bytes", media_header.size, rust("MEDIA_HEADER_LEN"), firmware("BOBE_MEDIA_HEADER_BYTES"))
contract.expect(
    "HIL microphone media kind",
    hil("MEDIA_MICROPHONE_PCM"),
    contract.integer("firmware BOBE_MEDIA_MICROPHONE_PCM", contract.c_enum_value(C_HEADER, "BOBE_MEDIA_MICROPHONE_PCM")),
)
contract.expect(
    "HIL speaker media kind",
    hil("MEDIA_SPEAKER_PCM"),
    contract.integer("firmware BOBE_MEDIA_SPEAKER_PCM", contract.c_enum_value(C_HEADER, "BOBE_MEDIA_SPEAKER_PCM")),
)
contract.expect(
    "HIL discontinuity flag",
    hil("MEDIA_FLAG_DISCONTINUITY"),
    rust("MEDIA_FLAG_DISCONTINUITY"),
    firmware("BOBE_MEDIA_FLAG_DISCONTINUITY"),
)
contract.expect("HIL microphone payload bytes", hil("MICROPHONE_PAYLOAD_BYTES"), rust("MIC_PCM_BYTES"), mic_bytes)
contract.expect("HIL speaker samples per frame", hil("SPEAKER_SAMPLES_PER_FRAME"), speaker_samples)
contract.expect("HIL speaker payload bytes", hil("SPEAKER_PAYLOAD_BYTES"), rust("SPEAKER_PCM_BYTES"), speaker_bytes)
contract.expect(
    "HIL speaker sample rate",
    hil("SPEAKER_SAMPLE_RATE"),
    rust("SPEAKER_SAMPLE_RATE_HZ"),
    firmware("BOBE_SPEAKER_SAMPLE_RATE_HZ", C_TYPES),
)
contract.expect(
    "HIL frame duration",
    hil("FRAME_DURATION_MS"),
    rust("AUDIO_FRAME_DURATION_MS"),
    firmware("BOBE_AUDIO_FRAME_DURATION_MS", C_TYPES),
)
contract.expect(
    "HIL control limit",
    hil("MAX_CONTROL_BYTES"),
    rust("MAX_CONTROL_BYTES"),
    firmware("BOBE_CONTROL_MAX_BYTES", C_TYPES),
)
contract.expect(
    "HIL playback credit",
    hil("MAX_PLAYBACK_CREDIT_MS"),
    rust("MAX_PLAYBACK_CREDIT_MS"),
    firmware("BOBE_MAX_PLAYBACK_CREDIT_MS"),
)

hil_sent = set()
hil_received = set()
for node in ast.walk(ast.parse(contract.read(PYTHON_HIL))):
    if (
        isinstance(node, ast.Call)
        and isinstance(node.func, ast.Attribute)
        and node.func.attr in {"send_control", "receive_control"}
        and node.args
        and isinstance(node.args[0], ast.Constant)
        and isinstance(node.args[0].value, str)
    ):
        (hil_sent if node.func.attr == "send_control" else hil_received).add(node.args[0].value)
    elif (
        isinstance(node, ast.Compare)
        and isinstance(node.left, ast.Subscript)
        and isinstance(node.left.slice, ast.Constant)
        and node.left.slice.value == "type"
        and len(node.ops) == 1
        and isinstance(node.ops[0], ast.Eq)
        and isinstance(node.comparators[0], ast.Constant)
        and isinstance(node.comparators[0].value, str)
    ):
        hil_received.add(node.comparators[0].value)

daemon_sources = "\n".join(contract.rust_production(path) for path in (RUST, RUST_GATEWAY, RUST_HANDLERS, RUST_UPDATE))
daemon_sent = set(contract.findall("daemon BodyLink message types", rf'(?:r#type|"type")\s*:\s*"({MESSAGE})"', daemon_sources))
handlers = contract.rust_production(RUST_HANDLERS)
daemon_handled = set(
    contract.findall("daemon handshake message type", rf'control_type\([^)]*\)\s*[!=]=\s*Some\("({MESSAGE})"\)', handlers)
)
for arm in contract.findall(
    "daemon BodyLink message dispatch arms",
    rf'^\s*("{MESSAGE}"(?:\s*\|\s*"{MESSAGE}")*)\s*=>',
    handlers,
):
    daemon_handled.update(re.findall(rf'"({MESSAGE})"', arm))

firmware_sources = control + "\n" + contract.read(C_INTERACTION)
firmware_sent = set()
for argument in contract.findall(
    "firmware BodyLink message builders",
    r'add_string\(\s*root\s*,\s*"type"\s*,\s*([^()]*?)\s*\)',
    firmware_sources,
):
    firmware_sent.update(re.findall(rf'"({MESSAGE})"', argument))
firmware_sent.update(re.findall(rf'\bmessage\("({MESSAGE})"\)', firmware_sources))
firmware_parsed = set(
    contract.findall(
        "firmware BodyLink message parsers",
        rf'strcmp\(\s*type->valuestring\s*,\s*"({MESSAGE})"\s*\)',
        firmware_sources,
    )
)
contract.expect_subset("HIL server messages", hil_sent, daemon_sent, "the WB-12 HIL server", "the daemon")
contract.expect_subset("HIL server messages", hil_sent, firmware_parsed, "the WB-12 HIL server", "the firmware parser")
contract.expect_subset("HIL device messages", hil_received, daemon_handled, "the WB-12 HIL server", "the daemon handler")
contract.expect_subset("HIL device messages", hil_received, firmware_sent, "the WB-12 HIL server", "the firmware")

contract.finish("BodyLink Rust/Swift/ESP/HIL contract constants match.")
PY
