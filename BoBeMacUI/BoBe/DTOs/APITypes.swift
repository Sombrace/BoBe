import Foundation


enum EventType: String, Codable, Sendable {
    case indicator
    case textDelta = "text_delta"
    case toolCallStart = "tool_call_start"
    case toolCallComplete = "tool_call_complete"
    case error
    case conversationClosed = "conversation_closed"
    case conversationChanged = "conversation_changed"
    case snapshotRequired = "snapshot_required"
    case bodyRequest = "body_request"
    case unknown

    init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = EventType(rawValue: raw) ?? .unknown
    }
}

struct StreamBundle: Codable, Sendable {
    var channelId: UUID? = nil
    let eventId: UInt64?
    let runtimeIncarnationId: UUID?
    let audience: String?
    let runId: UUID?
    let turnId: UUID?
    let type: EventType
    let payload: AnyCodablePayload
    let messageId: String

    enum CodingKeys: String, CodingKey {
        case channelId = "channel_id"
        case eventId = "event_id"
        case runtimeIncarnationId = "runtime_incarnation_id"
        case audience
        case runId = "run_id"
        case turnId = "turn_id"
        case type
        case payload
        case messageId = "message_id"
    }
}

struct SnapshotRequiredPayload: Codable, Sendable {
    let from: UInt64?
    let to: UInt64?
    let resumeAfter: UInt64?
    let reason: String
    let snapshotRequired: Bool

    enum CodingKeys: String, CodingKey {
        case from, to, reason
        case resumeAfter = "resume_after"
        case snapshotRequired = "snapshot_required"
    }
}

enum IndicatorType: String, Codable, Sendable, Equatable {
    case idle = "IDLE"
    case thinking = "THINKING"
    case streaming = "STREAMING"
    case unknown

    init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = IndicatorType(rawValue: raw) ?? .unknown
    }
}

struct IndicatorPayload: Codable, Sendable {
    let indicator: IndicatorType
}

struct TextDeltaPayload: Codable, Sendable {
    let delta: String
    let done: Bool
}

struct ToolCallStartPayload: Codable, Sendable {
    let status: String
    let toolName: String
    let toolCallId: String

    enum CodingKeys: String, CodingKey {
        case status
        case toolName = "tool_name"
        case toolCallId = "tool_call_id"
    }
}

struct ToolCallCompletePayload: Codable, Sendable {
    let status: String
    let toolName: String
    let toolCallId: String
    let success: Bool

    enum CodingKeys: String, CodingKey {
        case status
        case toolName = "tool_name"
        case toolCallId = "tool_call_id"
        case success
    }
}

struct ErrorPayload: Codable, Sendable {
    let code: String?
    let trigger: String?
    let message: String
    let recoverable: Bool

    var sourceLabel: String {
        self.trigger ?? self.code ?? "error"
    }

    var isTriggerError: Bool {
        self.trigger != nil
    }
}

struct ConversationClosedPayload: Codable, Sendable {
    let conversationId: String
    let reason: String
    let turnCount: Int

    enum CodingKeys: String, CodingKey {
        case conversationId = "conversation_id"
        case reason
        case turnCount = "turn_count"
    }
}

struct ConversationChangedPayload: Codable, Sendable {
    let conversationId: String

    enum CodingKeys: String, CodingKey {
        case conversationId = "conversation_id"
    }
}


struct AnyCodablePayload: Codable, Sendable {
    let data: Data

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let dict = try? container.decode([String: AnyCodableValue].self) {
            self.data = try JSONEncoder().encode(dict)
        } else {
            self.data = Data()
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(self.data)
    }

    func decode<T: Decodable>(as type: T.Type) throws -> T {
        try JSONDecoder().decode(type, from: self.data)
    }
}

enum AnyCodableValue: Codable, Sendable {
    case string(String)
    case int(Int)
    case double(Double)
    case bool(Bool)
    case null
    case array([AnyCodableValue])
    case dictionary([String: AnyCodableValue])

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if let v = try? container.decode(Bool.self) {
            self = .bool(v)
        } else if let v = try? container.decode(Int.self) {
            self = .int(v)
        } else if let v = try? container.decode(Double.self) {
            self = .double(v)
        } else if let v = try? container.decode(String.self) {
            self = .string(v)
        } else if let v = try? container.decode([AnyCodableValue].self) {
            self = .array(v)
        } else if let v = try? container.decode([String: AnyCodableValue].self) {
            self = .dictionary(v)
        } else if container.decodeNil() {
            self = .null
        } else {
            throw DecodingError.dataCorrupted(
                .init(
                    codingPath: decoder.codingPath,
                    debugDescription: "Unsupported JSON value"
                )
            )
        }
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case let .string(v): try container.encode(v)
        case let .int(v): try container.encode(v)
        case let .double(v): try container.encode(v)
        case let .bool(v): try container.encode(v)
        case .null: try container.encodeNil()
        case let .array(v): try container.encode(v)
        case let .dictionary(v): try container.encode(v)
        }
    }
}
