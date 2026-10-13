import Foundation

enum SpeechSiteWire: String, Codable, Sendable {
    case endpoint
    case daemon
}

enum SpeechTransferWire: String, Codable, Sendable {
    case transcript
    case text
    case audio
}

enum SpeechBoundaryOwnerWire: String, Codable, Sendable {
    case endpoint
    case daemon
}

enum SpeechPlacementOverrideWire: String, Codable, Sendable {
    case auto
    case endpoint
    case daemon
}

struct SpeechEngineOfferWire: Codable, Sendable {
    let id: String
    let languages: [String]
    let partials: Bool
    let eou: Bool
    let voices: [String]
    let sampleRates: [UInt32]
    let ready: Bool

    enum CodingKeys: String, CodingKey {
        case id, languages, partials, eou, voices, ready
        case sampleRates = "sample_rates"
    }
}

struct SpeechOverridesWire: Codable, Sendable {
    let stt: SpeechPlacementOverrideWire
    let tts: SpeechPlacementOverrideWire
}

struct SpeechOfferWire: Codable, Sendable {
    let endpointId: UUID
    let endpointKind: String
    let language: String
    let playbackRates: [UInt32]
    let endpointStt: [SpeechEngineOfferWire]
    let endpointTts: [SpeechEngineOfferWire]
    let audioUplink: Bool
    let overrides: SpeechOverridesWire
    let daemonIsNonLoopback: Bool
    let remoteAudioConsent: Bool

    enum CodingKeys: String, CodingKey {
        case language, overrides
        case endpointId = "endpoint_id"
        case endpointKind = "endpoint_kind"
        case playbackRates = "playback_rates"
        case endpointStt = "endpoint_stt"
        case endpointTts = "endpoint_tts"
        case audioUplink = "audio_uplink"
        case daemonIsNonLoopback = "daemon_is_non_loopback"
        case remoteAudioConsent = "remote_audio_consent"
    }
}

struct SpeechPlanWire: Codable, Sendable, Equatable {
    let planId: String
    let sttSite: SpeechSiteWire
    let sttEngineId: String
    let ttsSite: SpeechSiteWire
    let ttsEngineId: String
    let voiceId: String
    let uplink: SpeechTransferWire
    let downlink: SpeechTransferWire
    let eouOwner: SpeechBoundaryOwnerWire
    let wakeOwner: SpeechBoundaryOwnerWire
    let reason: String

    enum CodingKeys: String, CodingKey {
        case uplink, downlink, reason
        case planId = "plan_id"
        case sttSite = "stt_site"
        case sttEngineId = "stt_engine_id"
        case ttsSite = "tts_site"
        case ttsEngineId = "tts_engine_id"
        case voiceId = "voice_id"
        case eouOwner = "eou_owner"
        case wakeOwner = "wake_owner"
    }
}
