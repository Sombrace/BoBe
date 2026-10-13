import Foundation

struct AuthStatusResponse: Codable, Sendable {
    let isAuthenticated: Bool
    let authType: String?
    let host: String?
    let login: String?
    let statusMessage: String?

    enum CodingKeys: String, CodingKey {
        case isAuthenticated = "is_authenticated"
        case authType = "auth_type"
        case login, host
        case statusMessage = "status_message"
    }
}


struct LocalRuntimeInstallRequest: Codable, Sendable {
    let profile: String
    let chatModel: String
    let batchModel: String

    enum CodingKeys: String, CodingKey {
        case profile
        case chatModel = "chat_model"
        case batchModel = "batch_model"
    }
}

struct LocalRuntimeSnapshot: Codable, Sendable {
    let status: InstallStatusWire
    let error: String?
    let runtime: LocalRuntimeDownload
    let chatModel: LocalRuntimePull
    let batchModel: LocalRuntimePull

    enum CodingKeys: String, CodingKey {
        case status
        case error
        case runtime
        case chatModel = "chat_model"
        case batchModel = "batch_model"
    }
}

struct LocalRuntimeDownload: Codable, Sendable {
    let percent: Int?
}

struct LocalRuntimePull: Codable, Sendable {
    let status: String
    let percent: Int?
}


struct VoiceInstallSnapshot: Codable, Sendable {
    let status: InstallStatusWire
    let error: String?
    let models: [VoiceModelProgress]
    let installed: VoiceInstallPresence

    var isRunning: Bool {
        self.status == .running
    }

    var isTerminal: Bool {
        [.complete, .canceled, .failed, .idle].contains(self.status)
    }
}

struct VoiceModelProgress: Codable, Sendable, Identifiable {
    let kind: String
    let label: String
    let status: String
    let bytesDownloaded: UInt64
    let bytesTotal: UInt64?
    let percent: Int?

    var id: String {
        self.kind
    }

    enum CodingKeys: String, CodingKey {
        case kind
        case label
        case status
        case bytesDownloaded = "bytes_downloaded"
        case bytesTotal = "bytes_total"
        case percent
    }
}

struct VoiceInstallPresence: Codable, Sendable {
    let tts: Bool
}


enum ProviderLoginPhase: Equatable, Sendable {
    case preparing
    case awaitingUser(url: String, code: String)
    case polling(url: String, code: String)
    case awaitingInput(message: String, password: Bool)
    case completed
    case failed(message: String)
    case canceled
    case unsupported(phase: String)

    var isTerminal: Bool {
        switch self {
        case .completed, .failed, .canceled, .unsupported: true
        default: false
        }
    }
}

extension ProviderLoginPhase: Decodable {
    private enum CodingKeys: String, CodingKey {
        case phase
        case url
        case code
        case message
        case password
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let phase = try container.decode(String.self, forKey: .phase)
        switch phase {
        case "preparing":
            self = .preparing
        case "awaiting_user":
            self = try .awaitingUser(
                url: container.decode(String.self, forKey: .url),
                code: container.decode(String.self, forKey: .code)
            )
        case "polling":
            self = try .polling(
                url: container.decode(String.self, forKey: .url),
                code: container.decode(String.self, forKey: .code)
            )
        case "awaiting_input":
            self = try .awaitingInput(
                message: container.decode(String.self, forKey: .message),
                password: container.decode(Bool.self, forKey: .password)
            )
        case "completed":
            self = .completed
        case "failed":
            self = try .failed(message: container.decode(String.self, forKey: .message))
        case "canceled":
            self = .canceled
        default:
            self = .unsupported(phase: phase)
        }
    }
}
