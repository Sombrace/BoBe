#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUST="$ROOT/BoBeService/src/constants.rs"
SWIFT_CONST="$ROOT/BoBeMacUI/BoBe/App/Constants.swift"
SWIFT_ATTACH_WIRE="$ROOT/BoBeMacUI/BoBe/DTOs/AttachTypes.swift"
SWIFT_PROVIDER="$ROOT/BoBeMacUI/BoBe/DTOs/ProviderTypes.swift"
RUST_PROVIDER="$ROOT/BoBeService/src/provider/profile.rs"
RUST_SPEECH_PLAN="$ROOT/BoBeService/src/speech/plan.rs"
RUST_SCOPE="$ROOT/BoBeService/src/api/scope.rs"
SWIFT_ATTACH="$ROOT/BoBeMacUI/BoBe/Services/EndpointAttachService.swift"

fail=0

expect_match() {
    local name="$1" rust_value="$2" swift_value="$3"
    if [[ "$rust_value" != "$swift_value" ]]; then
        echo "drift: $name — Rust='$rust_value' Swift='$swift_value'" >&2
        fail=1
    fi
}

pascal_to_snake() {
    sed -E 's/([a-z0-9])([A-Z])/\1_\2/g' | tr '[:upper:]' '[:lower:]'
}

rust_enum_variants() {
    local file="$1"
    local enum_name="$2"
    awk "/pub\\(crate\\) enum $enum_name \\{/,/^}/" "$file" \
        | grep -oE '^    [A-Z][a-zA-Z0-9]+' \
        | sed -E 's/^    //' \
        | pascal_to_snake \
        | sort -u
}

swift_enum_variants() {
    local file="$1"
    local enum_name="$2"
    awk "/enum $enum_name:/,/^}/" "$file" \
        | sed -nE 's/^[[:space:]]*case ([a-zA-Z0-9_]+)( = "([^"]+)")?$/\1|\3/p' \
        | while IFS='|' read -r name wire; do
            if [[ -n "$wire" ]]; then
                printf '%s\n' "$wire"
            else
                printf '%s\n' "$name" | pascal_to_snake
            fi
        done \
        | sort -u
}

diff_sets() {
    local name="$1" rust="$2" swift="$3"
    if [[ "$rust" != "$swift" ]]; then
        echo "drift: $name variant set differs between Rust and Swift" >&2
        diff <(echo "$rust") <(echo "$swift") >&2 || true
        fail=1
    fi
}

rust_port=$(grep -E 'DEFAULT_DAEMON_PORT: u16 =' "$RUST" | sed -E 's/.*= ([0-9]+);.*/\1/')
swift_port=$(grep -E 'static let defaultPort = ' "$SWIFT_CONST" | sed -E 's/.*= ([0-9]+).*/\1/')
expect_match "daemon port" "$rust_port" "$swift_port"

rust_attach_min=$(grep -E 'PROTOCOL_MIN: u32 =' "$RUST" | sed -E 's/.*= ([0-9]+);.*/\1/')
swift_attach_min=$(grep -E 'static let protocolMin: UInt32 =' "$SWIFT_ATTACH_WIRE" | sed -E 's/.*= ([0-9]+).*/\1/')
expect_match "attach protocol minimum" "$rust_attach_min" "$swift_attach_min"
rust_attach_max=$(grep -E 'PROTOCOL_MAX: u32 =' "$RUST" | sed -E 's/.*= ([0-9]+);.*/\1/')
swift_attach_max=$(grep -E 'static let protocolMax: UInt32 =' "$SWIFT_ATTACH_WIRE" | sed -E 's/.*= ([0-9]+).*/\1/')
expect_match "attach protocol maximum" "$rust_attach_max" "$swift_attach_max"
rust_attach_path=$(grep -E 'pub\(crate\) const PATH: &str =' "$RUST" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_attach_path=$(grep -E 'static let path = "/v1/hello"' "$SWIFT_ATTACH_WIRE" | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "attach path" "$rust_attach_path" "$swift_attach_path"

rust_privacy_timeout=$(grep -E 'CLIENT_TIMEOUT_SECS: u64 =' "$RUST" | sed -E 's/.*= ([0-9]+);.*/\1/')
swift_privacy_timeout=$(grep -E 'purgeRequestTimeoutSeconds: TimeInterval =' "$SWIFT_CONST" | sed -E 's/.*= ([0-9]+).*/\1/')
expect_match "privacy purge Swift timeout" "$rust_privacy_timeout" "$swift_privacy_timeout"

rust_ollama_base=$(grep -E 'DEFAULT_OLLAMA_BASE_URL:' "$RUST" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_ollama_base=$(grep -E 'static let baseURL = "http://127' "$SWIFT_CONST" | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "ollama base URL" "$rust_ollama_base" "$swift_ollama_base"

rust_ollama_v1=$(grep -E 'DEFAULT_OLLAMA_V1_URL:' "$RUST" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_ollama_v1=$(grep -E 'static let v1URL = ' "$SWIFT_CONST" | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "ollama v1 URL" "$rust_ollama_v1" "$swift_ollama_v1"

diff_sets \
    "ProviderKind" \
    "$(rust_enum_variants "$RUST_PROVIDER" ProviderKind)" \
    "$(swift_enum_variants "$SWIFT_PROVIDER" ProviderKindWire)"
diff_sets \
    "LocalRuntimeKind" \
    "$(rust_enum_variants "$RUST_PROVIDER" LocalRuntimeKind)" \
    "$(swift_enum_variants "$SWIFT_PROVIDER" LocalRuntimeKindWire)"
diff_sets \
    "ProviderAuthMethod" \
    "$(rust_enum_variants "$RUST_PROVIDER" ProviderAuthMethod)" \
    "$(swift_enum_variants "$SWIFT_PROVIDER" ProviderAuthMethodWire)"
diff_sets \
    "AgentTool" \
    "$(rust_enum_variants "$ROOT/BoBeService/src/config/agent.rs" AgentTool)" \
    "$(swift_enum_variants "$ROOT/BoBeMacUI/BoBe/DTOs/AgentWorkTypes.swift" AgentToolWire)"
diff_sets \
    "AgentJobStatus" \
    "$(rust_enum_variants "$ROOT/BoBeService/src/db/agent_job_repo.rs" JobStatus)" \
    "$(swift_enum_variants "$ROOT/BoBeMacUI/BoBe/DTOs/AgentWorkTypes.swift" AgentJobStatusWire)"
diff_sets \
    "AgentRunKind" \
    "$(rust_enum_variants "$ROOT/BoBeService/src/runtime/run_registry.rs" RunKind)" \
    "$(swift_enum_variants "$ROOT/BoBeMacUI/BoBe/DTOs/AgentWorkTypes.swift" AgentRunKindWire)"
diff_sets \
    "OwnerInputKind" \
    "$(rust_enum_variants "$ROOT/BoBeService/src/db/owner_input_repo.rs" InputKind)" \
    "$(swift_enum_variants "$ROOT/BoBeMacUI/BoBe/DTOs/AgentWorkTypes.swift" OwnerInputKindWire)"
diff_sets \
    "OwnerInputStatus" \
    "$(rust_enum_variants "$ROOT/BoBeService/src/db/owner_input_repo.rs" InputStatus)" \
    "$(swift_enum_variants "$ROOT/BoBeMacUI/BoBe/DTOs/AgentWorkTypes.swift" OwnerInputStatusWire)"
diff_sets \
    "provider login phase" \
    "$(rust_enum_variants "$ROOT/BoBeService/src/agent/auth.rs" LoginPhase)" \
    "$(awk '/extension ProviderLoginPhase: Decodable/,/^}/' "$ROOT/BoBeMacUI/BoBe/DTOs/DaemonDTOs.swift" \
        | sed -nE 's/^[[:space:]]*case "([a-z_]+)":$/\1/p' | sort -u)"

extract_rust_engine() { grep -E "pub\(crate\) const $1: &str =" "$RUST" | sed -E 's/.*= "([^"]+)";.*/\1/'; }
extract_swift_engine() { grep -E "static let $1 = " "$SWIFT_CONST" | sed -E 's/.*= "([^"]+)".*/\1/'; }
expect_match "mcp_status::CONNECTED"      "$(extract_rust_engine CONNECTED)"      "$(extract_swift_engine connected)"
expect_match "mcp_status::FAILED"         "$(extract_rust_engine FAILED)"         "$(extract_swift_engine failed)"
expect_match "mcp_status::PENDING"        "$(extract_rust_engine PENDING)"        "$(extract_swift_engine pending)"
expect_match "mcp_status::STOPPED"        "$(extract_rust_engine STOPPED)"        "$(extract_swift_engine stopped)"

rust_sample=$(grep -E 'TTS_OUTPUT_SAMPLE_RATE: u32 =' "$RUST" | sed -E 's/.*= ([0-9_]+);.*/\1/' | tr -d '_')
swift_sample=$(grep -E 'static let ttsOutputSampleRate = ' "$SWIFT_CONST" | sed -E 's/.*= ([0-9_]+).*/\1/' | tr -d '_')
expect_match "voice TTS sample rate" "$rust_sample" "$swift_sample"

rust_kokoro=$(grep -E 'KOKORO_MODEL_DIR: &str =' "$RUST" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_kokoro=$(grep -E 'static let kokoroModelDir = ' "$SWIFT_CONST" | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "Kokoro model dir name" "$rust_kokoro" "$swift_kokoro"

rust_streaming_stt=$(grep -A1 -E 'STREAMING_STT_MODEL_DIR: &str =' "$RUST" | tail -n 1 | sed -E 's/.*"([^"]+)";.*/\1/')
swift_streaming_stt=$(grep -E 'static let streamingSttModelDir = ' "$SWIFT_CONST" | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "streaming STT model dir name" "$rust_streaming_stt" "$swift_streaming_stt"

rust_persona=$(grep -E 'DEFAULT_PERSONA: &str =' "$RUST" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_persona=$(grep -E 'static let defaultPersona = ' "$SWIFT_CONST" | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "voice default persona" "$rust_persona" "$swift_persona"

rust_kind_tts=$(grep -E 'MODEL_KIND_TTS: &str =' "$RUST" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_kind_tts=$(grep -E 'static let modelKindTts = ' "$SWIFT_CONST" | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "voice model kind TTS" "$rust_kind_tts" "$swift_kind_tts"

rust_kind_stt=$(grep -E 'MODEL_KIND_STT: &str =' "$RUST" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_kind_stt=$(grep -E 'static let modelKindStt = ' "$SWIFT_CONST" | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "voice model kind STT" "$rust_kind_stt" "$swift_kind_stt"

rust_subprotocol_v2=$(grep -E 'SUBPROTOCOL_V2: &str =' "$RUST" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_subprotocol_v2=$(grep -E 'static let subprotocolV2 = ' "$SWIFT_CONST" | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "voice WS subprotocol v2" "$rust_subprotocol_v2" "$swift_subprotocol_v2"

rust_apple_stt=$(grep -E 'APPLE_SPEECH_ANALYZER_ENGINE: &str =' "$RUST_SPEECH_PLAN" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_apple_stt=$(grep -E 'static let appleSpeechAnalyzer = ' "$SWIFT_CONST" | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "Apple Speech endpoint engine" "$rust_apple_stt" "$swift_apple_stt"

rust_parakeet_stt=$(grep -E 'FLUIDAUDIO_PARAKEET_ENGINE: &str =' "$RUST_SPEECH_PLAN" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_parakeet_stt=$(grep -E 'static let fluidAudioParakeet = ' "$SWIFT_CONST" | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "FluidAudio Parakeet endpoint engine" "$rust_parakeet_stt" "$swift_parakeet_stt"

rust_nemotron_stt=$(grep -E 'FLUIDAUDIO_NEMOTRON_ENGINE: &str =' "$RUST_SPEECH_PLAN" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_nemotron_stt=$(grep -E 'static let fluidAudioNemotron = ' "$SWIFT_CONST" | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "FluidAudio Nemotron endpoint engine" "$rust_nemotron_stt" "$swift_nemotron_stt"

rust_tc_start=$(grep -E 'pub\(crate\) const START: &str =' "$RUST" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_tc_start=$(grep -E 'static let start = "[^"]+"' "$SWIFT_CONST" | head -n 1 | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "tool_call_status::START" "$rust_tc_start" "$swift_tc_start"

rust_tc_complete=$(grep -E 'pub\(crate\) const COMPLETE: &str =' "$RUST" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_tc_complete=$(grep -E 'static let complete = "[^"]+"' "$SWIFT_CONST" | head -n 1 | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "tool_call_status::COMPLETE" "$rust_tc_complete" "$swift_tc_complete"

rust_data_dir=$(grep -E 'BOBE_DATA_DIR_NAME: &str =' "$RUST" | sed -E 's/.*= "([^"]+)";.*/\1/')
swift_data_dir=$(grep -E 'static let dataDirName = ' "$SWIFT_CONST" | sed -E 's/.*= "([^"]+)".*/\1/')
expect_match "BOBE_DATA_DIR_NAME" "$rust_data_dir" "$swift_data_dir"

rust_pause_tight=$(grep -E 'TIGHT: u32 =' "$RUST" | sed -E 's/.*= ([0-9]+);.*/\1/')
swift_pause_tight=$(grep -E 'static let tight: Int = ' "$SWIFT_CONST" | sed -E 's/.*= ([0-9]+).*/\1/')
expect_match "pause_sensitivity_ms::TIGHT" "$rust_pause_tight" "$swift_pause_tight"

rust_pause_balanced=$(grep -E 'BALANCED: u32 =' "$RUST" | sed -E 's/.*= ([0-9]+);.*/\1/')
swift_pause_balanced=$(grep -E 'static let balanced: Int = ' "$SWIFT_CONST" | sed -E 's/.*= ([0-9]+).*/\1/')
expect_match "pause_sensitivity_ms::BALANCED" "$rust_pause_balanced" "$swift_pause_balanced"

rust_pause_patient=$(grep -E 'PATIENT: u32 =' "$RUST" | sed -E 's/.*= ([0-9]+);.*/\1/')
swift_pause_patient=$(grep -E 'static let patient: Int = ' "$SWIFT_CONST" | sed -E 's/.*= ([0-9]+).*/\1/')
expect_match "pause_sensitivity_ms::PATIENT" "$rust_pause_patient" "$swift_pause_patient"

RUST_KOKORO="$ROOT/BoBeService/src/speech/providers/sherpa/kokoro_tts.rs"
SWIFT_KOKORO="$ROOT/BoBeMacUI/BoBe/Voice/VoiceSettingsEnums.swift"

rust_voices=$(grep -oE '"[a-z][a-z]_[a-z0-9]+"' "$RUST_KOKORO" | sort -u)
swift_voices=$(grep -oE '"[a-z][a-z]_[a-z0-9]+"' "$SWIFT_KOKORO" | sort -u)
rust_voice_count=$(echo "$rust_voices" | wc -l | tr -d ' ')
swift_voice_count=$(echo "$swift_voices" | wc -l | tr -d ' ')
expect_match "Kokoro voice slot count" "$rust_voice_count" "$swift_voice_count"
if [[ "$rust_voices" != "$swift_voices" ]]; then
    echo "drift: Kokoro voice set differs between Rust and Swift" >&2
    diff <(echo "$rust_voices") <(echo "$swift_voices") >&2 || true
    fail=1
fi

RUST_PROTO="$ROOT/BoBeService/src/speech/protocol.rs"
SWIFT_PROTO="$ROOT/BoBeMacUI/BoBe/Voice/VoiceProtocol.swift"


rust_owner_scopes=$(
    awk '/pub\(crate\) const fn as_str/,/^    }/' "$RUST_SCOPE" \
        | grep -oE '"[a-z_]+"' \
        | tr -d '"' \
        | sort -u
)
swift_owner_scopes=$(
    awk '/static let requiredScopes = \[/,/^    ]/' "$SWIFT_ATTACH" \
        | grep -oE '"[a-z_]+"' \
        | tr -d '"' \
        | sort -u
)
diff_sets "owner Scope::ALL" "$rust_owner_scopes" "$swift_owner_scopes"

RUST_BODY_CONFIG="$ROOT/BoBeService/src/config/body.rs"
SWIFT_SETTINGS_TYPES="$ROOT/BoBeMacUI/BoBe/DTOs/SettingsTypes.swift"
rust_body_themes=$(rust_enum_variants "$RUST_BODY_CONFIG" BodyTheme | tr '_' '-')
swift_body_themes=$(
    awk '/enum PhysicalBodyTheme: String/,/^}/' "$SWIFT_SETTINGS_TYPES" \
        | grep -oE '"[a-z]+(-[a-z]+)+"' \
        | tr -d '"' \
        | sort -u
)
diff_sets "physical BodyTheme" "$rust_body_themes" "$swift_body_themes"

rust_phase=$(rust_enum_variants "$RUST_PROTO" VoicePhase)
swift_phase=$(awk '/enum VoicePhaseWire: String, Codable \{/,/^}/' "$SWIFT_PROTO" \
    | grep -oE 'case [a-z][a-zA-Z0-9]+' \
    | sed -E 's/case //' \
    | sort -u)
diff_sets "VoicePhase" "$rust_phase" "$swift_phase"

rust_client=$(rust_enum_variants "$RUST_PROTO" ClientMessage)
swift_client=$(grep -oE 'c\.encode\("[a-z_]+", forKey: \.type\)' "$SWIFT_PROTO" \
    | sed -E 's/.*"([a-z_]+)".*/\1/' \
    | sort -u)
diff_sets "ClientMessage" "$rust_client" "$swift_client"

rust_server=$(rust_enum_variants "$RUST_PROTO" ServerMessage)
swift_server=$(awk '/enum ServerVoiceMessage: Decodable \{/,/^    \}/' "$SWIFT_PROTO" \
    | grep -oE 'case "[a-z_]+":' \
    | sed -E 's/case "([a-z_]+)":/\1/' \
    | sort -u)
diff_sets "ServerMessage" "$rust_server" "$swift_server"

RUST_SSE="$ROOT/BoBeService/src/util/sse/types.rs"
SWIFT_SSE="$ROOT/BoBeMacUI/BoBe/DTOs/APITypes.swift"

rust_events=$(rust_enum_variants "$RUST_SSE" EventType)
swift_events=$(
    awk '/enum EventType: String, Codable, Sendable \{/,/^}/' "$SWIFT_SSE" \
        | sed -nE 's/^[[:space:]]*case ([a-zA-Z0-9_]+)( = "([^"]+)")?$/\1|\3/p' \
        | while IFS='|' read -r name wire; do
            [[ "$name" == "unknown" ]] && continue
            if [[ -n "$wire" ]]; then
                printf '%s\n' "$wire"
            else
                printf '%s\n' "$name" | pascal_to_snake
            fi
        done \
        | sort -u
)
diff_sets "SSE EventType" "$rust_events" "$swift_events"

rust_indicators=$(rust_enum_variants "$RUST_SSE" IndicatorType | tr '[:lower:]' '[:upper:]')
swift_indicators=$(
    awk '/enum IndicatorType: String, Codable, Sendable, Equatable \{/,/^}/' "$SWIFT_SSE" \
        | grep -oE '"[A-Z_]+"' \
        | tr -d '"' \
        | grep -v '^UNKNOWN$' \
        | sort -u
)
diff_sets "SSE IndicatorType" "$rust_indicators" "$swift_indicators"

rust_header_len=$(grep -E 'TTS_FRAME_HEADER_LEN: usize =' "$RUST_PROTO" | sed -E 's/.*= ([0-9]+);.*/\1/')
swift_header_len=$(grep -E 'static let length = ' "$SWIFT_PROTO" | sed -E 's/.*= ([0-9]+).*/\1/')
expect_match "tts header length" "$rust_header_len" "$swift_header_len"

rust_flag_filler=$(grep -E 'FLAG_FILLER: u8 =' "$RUST_PROTO" | sed -E 's/.*= ([0-9b_]+);.*/\1/')
swift_flag_filler=$(grep -E 'static let flagFiller: UInt8 = ' "$SWIFT_PROTO" | sed -E 's/.*= ([0-9b_]+).*/\1/')
expect_match "tts FLAG_FILLER" "$rust_flag_filler" "$swift_flag_filler"

rust_flag_first=$(grep -E 'FLAG_FIRST_OF_TURN: u8 =' "$RUST_PROTO" | sed -E 's/.*= ([0-9b_]+);.*/\1/')
swift_flag_first=$(grep -E 'static let flagFirstOfTurn: UInt8 = ' "$SWIFT_PROTO" | sed -E 's/.*= ([0-9b_]+).*/\1/')
expect_match "tts FLAG_FIRST_OF_TURN" "$rust_flag_first" "$swift_flag_first"

if [[ $fail -eq 0 ]]; then
    echo "cross-language constants ok"
fi
exit $fail
