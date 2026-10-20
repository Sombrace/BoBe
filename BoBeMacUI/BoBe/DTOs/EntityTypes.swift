import Foundation

nonisolated enum GoalStatus: String, Codable, Sendable, CaseIterable {
    case active, paused, completed, archived, unknown

    init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = GoalStatus(rawValue: raw) ?? .unknown
    }
}

nonisolated struct Goal: Identifiable, Codable, Sendable {
    let id: String
    var title: String
    var status: GoalStatus
    var priority: Int
    var summary: String
    var whyItMatters: String
    var howWorkingOnIt: String
    var patternsObserved: String
    var attitudeFeelings: String
    var openQuestions: String
    var notes: String
    let createdAt: String
    var updatedAt: String

    enum CodingKeys: String, CodingKey {
        case id, title, status, priority, summary, notes
        case whyItMatters = "why_it_matters"
        case howWorkingOnIt = "how_working_on_it"
        case patternsObserved = "patterns_observed"
        case attitudeFeelings = "attitude_feelings"
        case openQuestions = "open_questions"
        case createdAt = "created_at"
        case updatedAt = "updated_at"
    }
}

nonisolated struct GoalListResponse: Codable, Sendable {
    let goals: [Goal]
    let activeCount: Int

    enum CodingKeys: String, CodingKey {
        case goals
        case activeCount = "active_count"
    }
}

nonisolated struct GoalCreateRequest: Codable, Sendable {
    let title: String
    var summary: String?
    var whyItMatters: String?
    var priority: Int?

    enum CodingKeys: String, CodingKey {
        case title, summary, priority
        case whyItMatters = "why_it_matters"
    }
}

nonisolated struct GoalUpdateRequest: Codable, Sendable {
    var title: String?
    var status: GoalStatus?
    var priority: Int?
    var summary: String?
    var whyItMatters: String?
    var notes: String?

    enum CodingKeys: String, CodingKey {
        case title, status, priority, summary, notes
        case whyItMatters = "why_it_matters"
    }
}

nonisolated struct MemoryResponse: Codable, Sendable {
    let content: String
    let bytes: Int
}

nonisolated struct MemoryUpdateRequest: Codable, Sendable {
    let content: String
}

nonisolated struct Soul: Identifiable, Codable, Sendable {
    let id: String
    var name: String
    var content: String
    var enabled: Bool
    let isDefault: Bool
    let createdAt: String
    var updatedAt: String

    enum CodingKeys: String, CodingKey {
        case id, name, content, enabled
        case isDefault = "is_default"
        case createdAt = "created_at"
        case updatedAt = "updated_at"
    }
}

nonisolated struct SoulListResponse: Codable, Sendable {
    let souls: [Soul]
    let count: Int
}

nonisolated struct SoulCreateRequest: Codable, Sendable {
    let name: String
    let content: String
    var enabled: Bool?
}

nonisolated struct SoulUpdateRequest: Codable, Sendable {
    var content: String?
    var enabled: Bool?
}

nonisolated struct UserProfile: Identifiable, Codable, Sendable {
    let id: String
    var name: String
    var content: String
    var enabled: Bool
    let isDefault: Bool
    let createdAt: String
    var updatedAt: String

    enum CodingKeys: String, CodingKey {
        case id, name, content, enabled
        case isDefault = "is_default"
        case createdAt = "created_at"
        case updatedAt = "updated_at"
    }
}

nonisolated struct UserProfileListResponse: Codable, Sendable {
    let profiles: [UserProfile]
}

nonisolated struct UserProfileCreateRequest: Codable, Sendable {
    let name: String
    let content: String
    var enabled: Bool?
}

nonisolated struct UserProfileUpdateRequest: Codable, Sendable {
    var content: String?
    var enabled: Bool?
}

nonisolated struct MCPServer: Identifiable, Codable, Sendable {
    var id: String {
        self.name
    }

    let name: String
    let command: String
    let args: [String]
    var status: String?
    var enabled: Bool
    var error: String?

    enum CodingKeys: String, CodingKey {
        case name, command, args, enabled, error, status
    }
}

nonisolated struct MCPConfigDocumentResponse: Codable, Sendable {
    let rawJson: String
    let servers: [MCPServer]

    enum CodingKeys: String, CodingKey {
        case servers
        case rawJson = "raw_json"
    }
}

nonisolated struct MCPConfigMutationRequest: Codable, Sendable {
    let rawJson: String

    enum CodingKeys: String, CodingKey {
        case rawJson = "raw_json"
    }
}

nonisolated struct MCPConfigValidateResponse: Codable, Sendable {
    let valid: Bool
    let normalizedJson: String
    let serverCount: Int
    let errors: [String]

    enum CodingKeys: String, CodingKey {
        case valid, errors
        case normalizedJson = "normalized_json"
        case serverCount = "server_count"
    }
}

nonisolated struct MCPConfigSaveResponse: Codable, Sendable {
    let message: String
    let rawJson: String
    let servers: [MCPServer]

    enum CodingKeys: String, CodingKey {
        case message, servers
        case rawJson = "raw_json"
    }
}

nonisolated struct MCPConfigResetResponse: Codable, Sendable {
    let message: String
    let rawJson: String

    enum CodingKeys: String, CodingKey {
        case message
        case rawJson = "raw_json"
    }
}
