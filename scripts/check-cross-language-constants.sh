#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PYTHONDONTWRITEBYTECODE=1 exec python3 - "$ROOT" <<'PY'
import sys
from pathlib import Path

sys.path.insert(0, str(Path(sys.argv[1]) / "scripts/lib"))
from source_contracts import Contract

contract = Contract(sys.argv[1], "cross-language")
rust_constants = "BoBeService/src/constants.rs"
rust_speech_plan = "BoBeService/src/speech/plan.rs"
rust_voice = "BoBeService/src/speech/protocol.rs"
rust_sse = "BoBeService/src/util/sse/types.rs"


def rust_integer(name, path=rust_constants, module=None):
    return contract.integer(f"Rust {name}", contract.rust_const(path, name, module))


def rust_string(name, path=rust_constants, module=None):
    return contract.string(f"Rust {name}", contract.rust_const(path, name, module))


def swift_integer(owner, name):
    return contract.integer(f"Swift {owner}.{name}", contract.swift_static(owner, name))


def swift_string(owner, name):
    return contract.string(f"Swift {owner}.{name}", contract.swift_static(owner, name))


def quoted(label, text, pattern=r'"([a-z_]+)"'):
    return set(contract.findall(label, pattern, text))


contract.expect("daemon port", rust_integer("DEFAULT_DAEMON_PORT"), swift_integer("DaemonConfig", "defaultPort"))
contract.expect(
    "attach protocol minimum",
    rust_integer("PROTOCOL_MIN", module="attach"),
    swift_integer("AttachWire", "protocolMin"),
)
contract.expect(
    "attach protocol maximum",
    rust_integer("PROTOCOL_MAX", module="attach"),
    swift_integer("AttachWire", "protocolMax"),
)
contract.expect("attach path", rust_string("PATH", module="attach"), swift_string("AttachWire", "path"))
contract.expect_set(
    "ReadinessStatus",
    contract.rust_enum("BoBeService/src/api/readiness.rs", "ReadinessStatus"),
    contract.swift_enum("ReadinessStateWire") - {"unknown"},
)

privacy_timeout = contract.rust_const(rust_constants, "HTTP_TIMEOUT", "privacy")
daemon_timeout = contract.integer(
    "Rust privacy HTTP_TIMEOUT",
    contract.find("Rust privacy HTTP_TIMEOUT seconds", r"Duration::from_secs\(([0-9_]+)\)$", privacy_timeout),
)
client_timeout = swift_integer("PrivacyWire", "purgeRequestTimeoutSeconds")
if daemon_timeout >= client_timeout:
    contract.drift(
        f"privacy purge daemon timeout {daemon_timeout}s must be below the Swift client timeout {client_timeout}s"
    )

contract.expect("ollama base URL", rust_string("DEFAULT_OLLAMA_BASE_URL"), swift_string("OllamaDefaults", "baseURL"))
contract.expect("ollama v1 URL", rust_string("DEFAULT_OLLAMA_V1_URL"), swift_string("OllamaDefaults", "v1URL"))

for label, rust_path, rust_name, swift_name in (
    ("ProviderKind", "BoBeService/src/provider/profile.rs", "ProviderKind", "ProviderKindWire"),
    ("LocalRuntimeKind", "BoBeService/src/provider/profile.rs", "LocalRuntimeKind", "LocalRuntimeKindWire"),
    ("ProviderAuthMethod", "BoBeService/src/provider/profile.rs", "ProviderAuthMethod", "ProviderAuthMethodWire"),
    ("AgentTool", "BoBeService/src/config/agent.rs", "AgentTool", "AgentToolWire"),
    ("AgentJobStatus", "BoBeService/src/db/agent_job_repo.rs", "JobStatus", "AgentJobStatusWire"),
    ("AgentRunKind", "BoBeService/src/runtime/run_registry.rs", "RunKind", "AgentRunKindWire"),
    ("OwnerInputStatus", "BoBeService/src/db/owner_input_repo.rs", "InputStatus", "OwnerInputStatusWire"),
    ("VoicePhase", rust_voice, "VoicePhase", "VoicePhaseWire"),
):
    contract.expect_set(label, contract.rust_enum(rust_path, rust_name), contract.swift_enum(swift_name))

login_phase = contract.single(
    "Swift ProviderLoginPhase decoder",
    [body for body in contract.swift_blocks("ProviderLoginPhase", ("extension",)) if "init(from decoder" in body],
)
contract.expect_set(
    "provider login phase",
    contract.rust_enum("BoBeService/src/agent/auth.rs", "LoginPhase"),
    quoted("Swift ProviderLoginPhase phases", login_phase, r'^\s*case "([a-z_]+)":$'),
)

for name in ("CONNECTED", "FAILED", "PENDING", "STOPPED"):
    contract.expect(
        f"mcp_status::{name}",
        rust_string(name, module="mcp_status"),
        swift_string("McpServerStatusWire", name.lower()),
    )
for name in ("START", "COMPLETE"):
    contract.expect(
        f"tool_call_status::{name}",
        rust_string(name, module="tool_call_status"),
        swift_string("ToolCallStatusWire", name.lower()),
    )
contract.expect("BOBE_DATA_DIR_NAME", rust_string("BOBE_DATA_DIR_NAME"), swift_string("BobePaths", "dataDirName"))

contract.expect(
    "voice TTS sample rate",
    rust_integer("TTS_OUTPUT_SAMPLE_RATE", module="voice_wire"),
    swift_integer("VoiceWire", "ttsOutputSampleRate"),
)
for label, rust_name, swift_name in (
    ("Pocket TTS model dir name", "POCKET_TTS_MODEL_DIR", "pocketTtsModelDir"),
    ("voice default persona", "DEFAULT_PERSONA", "defaultPersona"),
    ("voice model kind TTS", "MODEL_KIND_TTS", "modelKindTts"),
    ("voice WS subprotocol v2", "SUBPROTOCOL_V2", "subprotocolV2"),
):
    contract.expect(label, rust_string(rust_name, module="voice_wire"), swift_string("VoiceWire", swift_name))
contract.expect(
    "Apple Speech endpoint recognizer",
    rust_string("APPLE_SPEECH_ANALYZER_ENGINE", rust_speech_plan),
    swift_string("VoiceEngineId", "appleSpeechAnalyzer"),
)
contract.expect(
    "Apple system voice endpoint synthesizer",
    rust_string("APPLE_SPEECH_SYNTHESIZER_ENGINE", rust_speech_plan),
    swift_string("VoiceEngineId", "appleSpeechSynthesizer"),
)
contract.expect(
    "daemon Pocket TTS engine",
    rust_string("SERVER_POCKET_ENGINE", rust_speech_plan),
    swift_string("VoiceEngineId", "serverPocket"),
)

rust_voices = quoted(
    "Rust Pocket TTS voices",
    contract.rust_const("BoBeService/src/speech/providers/pocket.rs", "VOICES"),
    r'"([a-z]+)"',
)
swift_voices = quoted("Swift Pocket TTS voices", contract.swift_static("PocketVoices", "allSlots"), r'"([a-z]+)"')
contract.expect_set("Pocket TTS voices", rust_voices, swift_voices)

rust_scopes = quoted(
    "Rust owner scopes",
    contract.block(
        "Rust Scope::as_str",
        contract.read("BoBeService/src/api/scope.rs"),
        r"^[ \t]*pub\(crate\) const fn as_str\(self\) -> &'static str \{",
    ),
)
swift_scopes = quoted(
    "Swift required owner scopes",
    contract.find(
        "Swift requiredScopes",
        r"\bstatic let requiredScopes(?:\s*:\s*\[String\])?\s*=\s*\[([^\]]*)\]",
        contract.read("BoBeMacUI/BoBe/Services/EndpointAttachService.swift"),
    ),
)
contract.expect_set("owner Scope::ALL", rust_scopes, swift_scopes)

contract.expect_set(
    "physical BodyTheme",
    {theme.replace("_", "-") for theme in contract.rust_enum("BoBeService/src/config/body.rs", "BodyTheme")},
    contract.swift_enum("PhysicalBodyTheme"),
)

contract.expect_set(
    "ClientMessage",
    contract.rust_enum(rust_voice, "ClientMessage"),
    quoted(
        "Swift ClientVoiceMessage types",
        contract.swift_type("ClientVoiceMessage"),
        r'\bc\.encode\("([a-z_]+)", forKey: \.type\)',
    ),
)
contract.expect_set(
    "ServerMessage",
    contract.rust_enum(rust_voice, "ServerMessage"),
    quoted("Swift ServerVoiceMessage types", contract.swift_type("ServerVoiceMessage"), r'^\s*case "([a-z_]+)":$'),
)

contract.expect_set("SSE EventType", contract.rust_enum(rust_sse, "EventType"), contract.swift_enum("EventType") - {"unknown"})
contract.expect_set(
    "SSE IndicatorType",
    {indicator.upper() for indicator in contract.rust_enum(rust_sse, "IndicatorType")},
    contract.swift_enum("IndicatorType") - {"unknown"},
)

contract.expect("tts header length", rust_integer("TTS_FRAME_HEADER_LEN", rust_voice), swift_integer("TtsFrameHeader", "length"))
contract.expect("tts FLAG_FILLER", rust_integer("FLAG_FILLER", rust_voice), swift_integer("TtsFrameHeader", "flagFiller"))
contract.expect(
    "tts FLAG_FIRST_OF_TURN",
    rust_integer("FLAG_FIRST_OF_TURN", rust_voice),
    swift_integer("TtsFrameHeader", "flagFirstOfTurn"),
)

for label, rust_name, swift_name in (
    ("voice capture header bytes", "VOICE_CAPTURE_HEADER_LEN", "captureHeaderBytes"),
    ("voice capture payload bytes", "VOICE_CAPTURE_PCM_BYTES", "capturePayloadBytes"),
    ("voice capture sample rate", "VOICE_CAPTURE_SAMPLE_RATE", "captureSampleRate"),
    ("voice capture frame samples", "VOICE_CAPTURE_FRAME_SAMPLES", "captureFrameSamples"),
):
    contract.expect(label, rust_integer(rust_name, rust_voice), swift_integer("VoiceWire", swift_name))

contract.finish("cross-language constants ok")
PY
