import Foundation

nonisolated enum PhysicalBodyTheme: String, Codable, CaseIterable, Identifiable, Sendable {
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

nonisolated struct DaemonSettings: Codable, Sendable, Equatable {
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

nonisolated struct SettingsUpdateRequest: Codable, Sendable, Equatable {
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

nonisolated struct SettingsUpdateResponse: Codable, Sendable {
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

nonisolated struct SendMessageRequest: Codable, Sendable {
    let content: String
    let requestId: UUID

    enum CodingKeys: String, CodingKey {
        case content
        case requestId = "request_id"
    }
}

nonisolated struct SendMessageResponse: Codable, Sendable {
    let messageId: String
    let replayed: Bool

    enum CodingKeys: String, CodingKey {
        case messageId = "message_id"
        case replayed
    }
}

nonisolated enum InitializationStateWire: String, Codable, Sendable, Equatable {
    case providerRequired = "provider_required"
    case personalizationRequired = "personalization_required"
    case ready

    init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = InitializationStateWire(rawValue: raw) ?? .providerRequired
    }
}

nonisolated struct ReadinessResponse: Codable, Sendable {
    let runtimeIncarnationId: UUID

    enum CodingKeys: String, CodingKey {
        case runtimeIncarnationId = "runtime_incarnation_id"
    }
}

nonisolated struct InitializationResponse: Codable, Sendable {
    let initializationState: InitializationStateWire

    enum CodingKeys: String, CodingKey {
        case initializationState = "initialization_state"
    }
}

nonisolated struct InitializationAdvanceRequest: Codable, Sendable {
    let initializationState: InitializationStateWire

    enum CodingKeys: String, CodingKey {
        case initializationState = "initialization_state"
    }
}

nonisolated struct HealthResponse: Codable, Sendable {
    let services: HealthServices
}

nonisolated struct HealthServices: Codable, Sendable {
    let database: String
}

nonisolated struct StatusResponse: Codable, Sendable {
    let indicator: String
    let acceptingUserMessages: Bool

    enum CodingKeys: String, CodingKey {
        case indicator
        case acceptingUserMessages = "accepting_user_messages"
    }

    var indicatorType: IndicatorType {
        IndicatorType(rawValue: self.indicator) ?? .unknown
    }
}
