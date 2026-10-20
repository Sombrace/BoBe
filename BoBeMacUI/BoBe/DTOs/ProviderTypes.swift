import Foundation

nonisolated enum ProviderKindWire: String, Codable, Sendable, CaseIterable {
    case copilot
    case openAI = "open_ai"
    case openAICompatible = "open_ai_compatible"
    case anthropic
    case deepSeek = "deepseek"
    case openRouter = "open_router"
    case railRelay = "rail_relay"

    static let choices: [ProviderKindWire] = allCases.filter { $0 != .openAICompatible } + [.openAICompatible]

    var defaultAuthMethod: ProviderAuthMethodWire {
        self == .copilot ? .oauth : .apiKey
    }

    var supportsOAuth: Bool {
        [.copilot, .openAI].contains(self)
    }

    var profileID: String {
        switch self {
        case .copilot: "copilot"
        case .openAI: "openai"
        case .anthropic: "anthropic"
        case .deepSeek: "deepseek"
        case .openRouter: "openrouter"
        case .railRelay: "rail-relay"
        case .openAICompatible: "custom"
        }
    }

    var displayName: String {
        switch self {
        case .copilot: "GitHub Copilot"
        case .openAI: "OpenAI"
        case .anthropic: "Anthropic"
        case .deepSeek: "DeepSeek"
        case .openRouter: "OpenRouter"
        case .railRelay: "Rail Relay"
        case .openAICompatible: L10n.tr("settings.providers.compatible.title")
        }
    }
}

nonisolated enum ProviderAuthMethodWire: String, Codable, Sendable {
    case apiKey = "api_key"
    case oauth
}

nonisolated enum LocalRuntimeKindWire: String, Codable, Sendable {
    case ollama
}

nonisolated struct ProviderProfileWire: Codable, Identifiable, Sendable, Equatable {
    var models: ProviderModelsWire = .init()
    let id: String
    let kind: ProviderKindWire
    let authMethod: ProviderAuthMethodWire
    let baseURL: String?
    let organization: String?
    let offline: Bool
    let localRuntime: LocalRuntimeKindWire?
    let credentialPresent: Bool
    let restartRequired: Bool

    var runsOnAuthority: Bool {
        guard self.kind == .openAICompatible, let baseURL,
              let host = URLComponents(string: baseURL)?.host?.lowercased() else { return false }
        return ["localhost", "127.0.0.1", "[::1]", "::1"].contains(host)
    }

    var destination: String {
        self.baseURL.flatMap { URLComponents(string: $0)?.host } ?? self.kind.displayName
    }

    enum CodingKeys: String, CodingKey {
        case models
        case id, kind, organization, offline
        case authMethod = "auth_method"
        case baseURL = "base_url"
        case localRuntime = "local_runtime"
        case credentialPresent = "credential_present"
        case restartRequired = "restart_required"
    }

    struct ProviderModelsWire: Codable, Sendable, Equatable {
        var chat: String?
        var goals: String?
        var consolidate: String?
        var small: String?
    }

    init(
        id: String,
        kind: ProviderKindWire,
        authMethod: ProviderAuthMethodWire? = nil,
        baseURL: String? = nil,
        organization: String? = nil,
        offline: Bool = false,
        localRuntime: LocalRuntimeKindWire? = nil,
        credentialPresent: Bool = false,
        restartRequired: Bool = false
    ) {
        self.id = id
        self.kind = kind
        self.authMethod = authMethod ?? kind.defaultAuthMethod
        self.baseURL = baseURL
        self.organization = organization
        self.offline = offline
        self.localRuntime = localRuntime
        self.credentialPresent = credentialPresent
        self.restartRequired = restartRequired
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try container.decode(ProviderKindWire.self, forKey: .kind)
        self.init(
            id: try container.decode(String.self, forKey: .id),
            kind: kind,
            authMethod: try container.decode(ProviderAuthMethodWire.self, forKey: .authMethod),
            baseURL: try container.decodeIfPresent(String.self, forKey: .baseURL),
            organization: try container.decodeIfPresent(String.self, forKey: .organization),
            offline: try container.decode(Bool.self, forKey: .offline),
            localRuntime: try container.decodeIfPresent(LocalRuntimeKindWire.self, forKey: .localRuntime),
            credentialPresent: try container.decode(Bool.self, forKey: .credentialPresent),
            restartRequired: try container.decode(Bool.self, forKey: .restartRequired)
        )
        self.models = try container.decode(ProviderModelsWire.self, forKey: .models)
    }
}

nonisolated struct WorkloadRouteWire: Codable, Sendable, Equatable {
    let profile: String
    let model: String?
    let reasoningEffort: String?

    enum CodingKeys: String, CodingKey {
        case profile, model
        case reasoningEffort = "reasoning_effort"
    }
}

nonisolated enum ProviderWorkload: String, CaseIterable, Identifiable, Sendable {
    case chat, goals, small, consolidate

    var id: String { self.rawValue }
    var label: String {
        switch self {
        case .chat: L10n.tr("settings.providers.workload.chat")
        case .goals: L10n.tr("settings.providers.workload.goals")
        case .small: L10n.tr("settings.providers.workload.small")
        case .consolidate: L10n.tr("settings.providers.workload.consolidate")
        }
    }
    var symbol: String {
        switch self {
        case .chat: "bubble.left.and.bubble.right"
        case .goals: "target"
        case .small: "bolt"
        case .consolidate: "brain"
        }
    }
}

nonisolated struct ProviderRoutesWire: Codable, Sendable, Equatable {
    let chat: WorkloadRouteWire
    let goals: WorkloadRouteWire
    let consolidate: WorkloadRouteWire
    var small: WorkloadRouteWire?

    func route(for workload: ProviderWorkload) -> WorkloadRouteWire {
        switch workload {
        case .chat: self.chat
        case .goals: self.goals
        case .consolidate: self.consolidate
        case .small: self.small ?? self.goals
        }
    }
}

nonisolated struct ProvidersResponse: Codable, Sendable {
    let profiles: [ProviderProfileWire]
    let routes: ProviderRoutesWire
    let activeProfiles: [ProviderProfileWire]
    let activeRoutes: ProviderRoutesWire
    let restartRequired: Bool

    enum CodingKeys: String, CodingKey {
        case profiles, routes
        case activeProfiles = "active_profiles"
        case activeRoutes = "active_routes"
        case restartRequired = "restart_required"
    }

    func canRemove(_ profile: ProviderProfileWire) -> Bool {
        !ProviderWorkload.allCases.contains {
            self.routes.route(for: $0).profile == profile.id
                || self.activeRoutes.route(for: $0).profile == profile.id
        }
    }

    func isPending(_ workload: ProviderWorkload) -> Bool {
        let route = self.routes.route(for: workload)
        return route != self.activeRoutes.route(for: workload)
            || self.profiles.first(where: { $0.id == route.profile })?.restartRequired == true
    }
}

nonisolated struct ProviderProbeRequest: Encodable, Sendable {
    let model: String
    let reasoningEffort: String?
    enum CodingKeys: String, CodingKey {
        case model
        case reasoningEffort = "reasoning_effort"
    }
}

nonisolated struct ProviderProbeResponse: Decodable, Sendable {
    let ok: Bool
    let profile: String
    let model: String?
    let modelExecution: Bool
    let streaming: Bool
    let toolExecution: Bool
    let error: String?
    enum CodingKeys: String, CodingKey {
        case ok, profile, model, streaming, error
        case modelExecution = "model_execution"
        case toolExecution = "tool_execution"
    }
}

nonisolated struct ProviderActivationRequest: Codable, Sendable, Equatable {
    let kind: ProviderKindWire
    let authMethod: ProviderAuthMethodWire
    let baseURL: String?
    let organization: String?
    let offline: Bool
    let localRuntime: LocalRuntimeKindWire?
    let apiKey: String?
    let chatModel: String?
    let goalsModel: String?
    let consolidateModel: String?
    let smallModel: String?

    init(
        kind: ProviderKindWire,
        authMethod: ProviderAuthMethodWire? = nil,
        baseURL: String? = nil,
        organization: String? = nil,
        offline: Bool = false,
        localRuntime: LocalRuntimeKindWire? = nil,
        apiKey: String? = nil,
        chatModel: String? = nil,
        goalsModel: String? = nil,
        consolidateModel: String? = nil,
        smallModel: String? = nil
    ) {
        self.kind = kind
        self.authMethod = authMethod ?? kind.defaultAuthMethod
        self.baseURL = baseURL
        self.organization = organization
        self.offline = offline
        self.localRuntime = localRuntime
        self.apiKey = apiKey
        self.chatModel = chatModel
        self.goalsModel = goalsModel
        self.consolidateModel = consolidateModel
        self.smallModel = smallModel
    }

    static func ollama(
        chatModel: String,
        batchModel: String
    ) -> Self {
        Self(
            kind: .openAICompatible,
            baseURL: OllamaDefaults.v1URL,
            organization: nil,
            offline: true,
            localRuntime: .ollama,
            apiKey: nil,
            chatModel: chatModel,
            goalsModel: batchModel,
            consolidateModel: batchModel,
            smallModel: batchModel
        )
    }

    enum CodingKeys: String, CodingKey {
        case kind, organization, offline
        case authMethod = "auth_method"
        case baseURL = "base_url"
        case localRuntime = "local_runtime"
        case apiKey = "api_key"
        case chatModel = "chat_model"
        case goalsModel = "goals_model"
        case consolidateModel = "consolidate_model"
        case smallModel = "small_model"
    }
}

nonisolated struct ProviderUpsertRequest: Codable, Sendable {
    let kind: ProviderKindWire
    let authMethod: ProviderAuthMethodWire
    let baseURL: String?
    let organization: String?
    let offline: Bool
    let localRuntime: LocalRuntimeKindWire?

    init(_ request: ProviderActivationRequest) {
        self.kind = request.kind
        self.authMethod = request.authMethod
        self.baseURL = request.baseURL
        self.organization = request.organization
        self.offline = request.offline
        self.localRuntime = request.localRuntime
    }

    enum CodingKeys: String, CodingKey {
        case kind, organization, offline
        case authMethod = "auth_method"
        case baseURL = "base_url"
        case localRuntime = "local_runtime"
    }
}

nonisolated struct ProviderModelInfo: Decodable, Identifiable, Sendable {
    let id: String
    let name: String
    let vision: Bool
    let contextWindow: Int?
    let supportedReasoningEfforts: [String]?

    enum CodingKeys: String, CodingKey {
        case id, name, vision
        case contextWindow = "context_window"
        case supportedReasoningEfforts = "supported_reasoning_efforts"
    }

    static func effortLabel(_ effort: String) -> String {
        switch effort {
        case "off": L10n.tr("settings.providers.effort.off")
        case "minimal": L10n.tr("settings.providers.effort.minimal")
        case "low": L10n.tr("settings.providers.effort.low")
        case "medium": L10n.tr("settings.providers.effort.medium")
        case "high": L10n.tr("settings.providers.effort.high")
        case "xhigh": L10n.tr("settings.providers.effort.xhigh")
        case "max": L10n.tr("settings.providers.effort.max")
        default: effort
        }
    }
}

nonisolated struct ProviderModelsResponse: Decodable, Sendable {
    let models: [ProviderModelInfo]
}

nonisolated struct ProviderMutationResponse: Codable, Sendable {
    let restartRequired: Bool

    enum CodingKeys: String, CodingKey {
        case restartRequired = "restart_required"
    }
}
