import Foundation


enum GoalStatus: String, Codable, Sendable, CaseIterable {
    case active, paused, completed, archived, unknown

    init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = GoalStatus(rawValue: raw) ?? .unknown
    }
}

struct Goal: Identifiable, Codable, Sendable {
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

struct GoalListResponse: Codable, Sendable {
    let goals: [Goal]
    let count: Int
    let activeCount: Int

    enum CodingKeys: String, CodingKey {
        case goals, count
        case activeCount = "active_count"
    }
}

struct GoalCreateRequest: Codable, Sendable {
    let title: String
    var summary: String?
    var whyItMatters: String?
    var priority: Int?

    enum CodingKeys: String, CodingKey {
        case title, summary, priority
        case whyItMatters = "why_it_matters"
    }
}

struct GoalUpdateRequest: Codable, Sendable {
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


struct MemoryResponse: Codable, Sendable {
    let content: String
    let bytes: Int
}

struct MemoryUpdateRequest: Codable, Sendable {
    let content: String
}


struct Soul: Identifiable, Codable, Sendable {
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

struct SoulListResponse: Codable, Sendable {
    let souls: [Soul]
    let count: Int
    let enabledCount: Int

    enum CodingKeys: String, CodingKey {
        case souls, count
        case enabledCount = "enabled_count"
    }
}

struct SoulCreateRequest: Codable, Sendable {
    let name: String
    let content: String
    var enabled: Bool?
}

struct SoulUpdateRequest: Codable, Sendable {
    var content: String?
    var enabled: Bool?
}


struct UserProfile: Identifiable, Codable, Sendable {
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

struct UserProfileListResponse: Codable, Sendable {
    let profiles: [UserProfile]
    let count: Int
    let enabledCount: Int

    enum CodingKeys: String, CodingKey {
        case profiles, count
        case enabledCount = "enabled_count"
    }
}

struct UserProfileCreateRequest: Codable, Sendable {
    let name: String
    let content: String
    var enabled: Bool?
}

struct UserProfileUpdateRequest: Codable, Sendable {
    var content: String?
    var enabled: Bool?
}


struct MCPServer: Identifiable, Codable, Sendable {
    var id: String {
        self.name
    }

    let name: String
    let command: String
    let args: [String]
    var connected: Bool
    var status: String?
    var enabled: Bool
    var error: String?

    enum CodingKeys: String, CodingKey {
        case name, command, args, connected, enabled, error, status
    }
}

struct MCPConfigDocumentResponse: Codable, Sendable {
    let rawJson: String
    let servers: [MCPServer]
    let count: Int
    let connectedCount: Int

    enum CodingKeys: String, CodingKey {
        case servers, count
        case rawJson = "raw_json"
        case connectedCount = "connected_count"
    }
}

struct MCPConfigMutationRequest: Codable, Sendable {
    let rawJson: String
    var secretKeys: [String: [String]]?

    enum CodingKeys: String, CodingKey {
        case rawJson = "raw_json"
        case secretKeys = "secret_keys"
    }
}

struct MCPConfigValidateResponse: Codable, Sendable {
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

struct MCPConfigSaveResponse: Codable, Sendable {
    let message: String
    let rawJson: String
    let servers: [MCPServer]
    let count: Int
    let connectedCount: Int

    enum CodingKeys: String, CodingKey {
        case message, servers, count
        case rawJson = "raw_json"
        case connectedCount = "connected_count"
    }
}

struct MCPConfigResetResponse: Codable, Sendable {
    let message: String
    let rawJson: String
    let count: Int

    enum CodingKeys: String, CodingKey {
        case message, count
        case rawJson = "raw_json"
    }
}
