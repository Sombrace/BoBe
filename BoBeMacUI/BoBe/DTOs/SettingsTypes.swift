import Foundation


enum PhysicalBodyTheme: String, Codable, CaseIterable, Identifiable, Sendable {
    case warmAperture = "warm-aperture"
    case warmLintel = "warm-lintel"
    case warmLedge = "warm-ledge"
    case kettleTallBright = "kettle-tall-bright"
    case kettleSoftBowl = "kettle-soft-bowl"
    case solarWideAwake = "solar-wide-awake"
    case solarCrown = "solar-crown"

    var id: String { self.rawValue }

    var name: String {
        L10n.tr("settings.appearance.body.theme.\(self.rawValue).name")
    }
}

struct DaemonSettings: Codable, Sendable {
    var bodyTheme: PhysicalBodyTheme
    var bodyConnected: Bool
    var captureEnabled: Bool
    var captureIntervalSeconds: Int
    var checkinEnabled: Bool
    var checkinTimes: [String]
    var checkinJitterMinutes: Int
    var conversationInactivityTimeoutSeconds: Int
    var conversationAutoCloseMinutes: Int
    var goalCheckIntervalSeconds: Double
    var mcpEnabled: Bool
    var voiceEnabled: Bool
    var voicePersona: String
    var voiceSpeed: Float
    var voiceSttLanguage: String
    var voicePauseSensitivity: String
    var voiceShowPartialCaption: Bool

    enum CodingKeys: String, CodingKey {
        case bodyTheme = "body_theme"
        case bodyConnected = "body_connected"
        case captureEnabled = "capture_enabled"
        case captureIntervalSeconds = "capture_interval_seconds"
        case checkinEnabled = "checkin_enabled"
        case checkinTimes = "checkin_times"
        case checkinJitterMinutes = "checkin_jitter_minutes"
        case conversationInactivityTimeoutSeconds = "conversation_inactivity_timeout_seconds"
        case conversationAutoCloseMinutes = "conversation_auto_close_minutes"
        case goalCheckIntervalSeconds = "goal_check_interval_seconds"
        case mcpEnabled = "mcp_enabled"
        case voiceEnabled = "voice_enabled"
        case voicePersona = "voice_persona"
        case voiceSpeed = "voice_speed"
        case voiceSttLanguage = "voice_stt_language"
        case voicePauseSensitivity = "voice_pause_sensitivity"
        case voiceShowPartialCaption = "voice_show_partial_caption"
    }
}

struct SettingsUpdateRequest: Codable, Sendable {
    var bodyTheme: PhysicalBodyTheme?
    var captureEnabled: Bool?
    var captureIntervalSeconds: Int?
    var checkinEnabled: Bool?
    var checkinTimes: [String]?
    var checkinJitterMinutes: Int?
    var conversationInactivityTimeoutSeconds: Int?
    var conversationAutoCloseMinutes: Int?
    var goalCheckIntervalSeconds: Double?
    var mcpEnabled: Bool?
    var voiceEnabled: Bool?
    var voicePersona: String?
    var voiceSpeed: Float?
    var voiceSttLanguage: String?
    var voicePauseSensitivity: String?
    var voiceShowPartialCaption: Bool?

    enum CodingKeys: String, CodingKey {
        case bodyTheme = "body_theme"
        case captureEnabled = "capture_enabled"
        case captureIntervalSeconds = "capture_interval_seconds"
        case checkinEnabled = "checkin_enabled"
        case checkinTimes = "checkin_times"
        case checkinJitterMinutes = "checkin_jitter_minutes"
        case conversationInactivityTimeoutSeconds = "conversation_inactivity_timeout_seconds"
        case conversationAutoCloseMinutes = "conversation_auto_close_minutes"
        case goalCheckIntervalSeconds = "goal_check_interval_seconds"
        case mcpEnabled = "mcp_enabled"
        case voiceEnabled = "voice_enabled"
        case voicePersona = "voice_persona"
        case voiceSpeed = "voice_speed"
        case voiceSttLanguage = "voice_stt_language"
        case voicePauseSensitivity = "voice_pause_sensitivity"
        case voiceShowPartialCaption = "voice_show_partial_caption"
    }
}

struct SettingsUpdateResponse: Codable, Sendable {
    let message: String
    let restartRequiredFields: [String]
    var persistFailed: Bool?
    var bodyThemeDelivered: Bool?

    enum CodingKeys: String, CodingKey {
        case message
        case restartRequiredFields = "restart_required_fields"
        case persistFailed = "persist_failed"
        case bodyThemeDelivered = "body_theme_delivered"
    }
}


struct SendMessageRequest: Codable, Sendable {
    let content: String
    let requestId: UUID

    enum CodingKeys: String, CodingKey {
        case content
        case requestId = "request_id"
    }
}

struct SendMessageResponse: Codable, Sendable {
    let messageId: String
    let requestStatus: String
    let replayed: Bool

    enum CodingKeys: String, CodingKey {
        case messageId = "message_id"
        case requestStatus = "request_status"
        case replayed
    }
}

enum InitializationStateWire: String, Codable, Sendable, Equatable {
    case providerRequired = "provider_required"
    case personalizationRequired = "personalization_required"
    case ready
}

struct ReadinessResponse: Codable, Sendable {
    let status: String
    let runtimeAuthorityId: UUID
    let runtimeAuthorityEpoch: Int
    let runtimeIncarnationId: UUID
    let daemonVersion: String
    let schemaVersion: Int
    let initializationState: InitializationStateWire
    let startedAt: String
    let blockingReasons: [String]

    enum CodingKeys: String, CodingKey {
        case status
        case runtimeAuthorityId = "runtime_authority_id"
        case runtimeAuthorityEpoch = "runtime_authority_epoch"
        case runtimeIncarnationId = "runtime_incarnation_id"
        case daemonVersion = "daemon_version"
        case schemaVersion = "schema_version"
        case initializationState = "initialization_state"
        case startedAt = "started_at"
        case blockingReasons = "blocking_reasons"
    }
}

struct InitializationResponse: Codable, Sendable {
    let initializationState: InitializationStateWire

    enum CodingKeys: String, CodingKey {
        case initializationState = "initialization_state"
    }
}

struct InitializationAdvanceRequest: Codable, Sendable {
    let initializationState: InitializationStateWire

    enum CodingKeys: String, CodingKey {
        case initializationState = "initialization_state"
    }
}

struct HealthResponse: Codable, Sendable {
    let status: String
    let services: HealthServices
}

struct HealthServices: Codable, Sendable {
    let database: String
}

struct StatusResponse: Codable, Sendable {
    let indicator: String
    let capturing: Bool
    let acceptingUserMessages: Bool

    enum CodingKeys: String, CodingKey {
        case indicator, capturing
        case acceptingUserMessages = "accepting_user_messages"
    }

    var indicatorType: IndicatorType {
        IndicatorType(rawValue: self.indicator) ?? .unknown
    }
}
