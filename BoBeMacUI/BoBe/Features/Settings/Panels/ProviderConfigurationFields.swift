import Foundation

struct ProviderConfigurationFields: Equatable {
    var kind: ProviderKindWire
    var authMethod: ProviderAuthMethodWire
    var profileID: String
    var baseURL = ""
    var organization = ""
    var offline = false
    var localRuntime: LocalRuntimeKindWire?
    var credentialPresent = false
    var isConfigured = false
    var isAuthenticated = false
    var chat = ""
    var batch = ""
    var small = ""
    var consolidate = ""

    init(kind: ProviderKindWire = .openAI) {
        self.kind = kind
        self.authMethod = kind.defaultAuthMethod
        self.profileID = kind.profileID
    }

    var needsBaseURL: Bool {
        [.openAICompatible, .railRelay].contains(self.kind)
    }

    var hasValidProfile: Bool {
        self.profileID.wholeMatch(of: /[a-z0-9][a-z0-9_-]{0,31}/) != nil
            && (!self.needsBaseURL || Self.validBaseURL(self.baseURL))
    }

    static func availableID(_ preferred: String, existing: Set<String>) -> String {
        guard existing.contains(preferred) else { return preferred }
        var suffix = 2
        while existing.contains("\(preferred.prefix(26))-\(suffix)") { suffix += 1 }
        return "\(preferred.prefix(26))-\(suffix)"
    }

    func canConnect(apiKey: String) -> Bool {
        self.hasValidProfile && (self.authMethod == .oauth
            || (self.kind == .openAICompatible && self.offline)
            || self.credentialPresent || Self.nonempty(apiKey) != nil)
    }

    func canActivate(apiKey: String) -> Bool {
        self.canConnect(apiKey: apiKey) && Self.nonempty(self.chat) != nil
            && (self.authMethod != .oauth || self.isAuthenticated)
    }

    func activation(apiKey: String) -> ProviderActivationRequest {
        ProviderActivationRequest(
            kind: self.kind,
            authMethod: self.authMethod,
            baseURL: Self.nonempty(self.baseURL),
            organization: Self.nonempty(self.organization),
            offline: self.offline,
            localRuntime: self.localRuntime,
            apiKey: self.authMethod == .apiKey ? Self.nonempty(apiKey) : nil,
            chatModel: Self.nonempty(self.chat),
            goalsModel: Self.nonempty(self.batch) ?? Self.nonempty(self.chat),
            consolidateModel: Self.nonempty(self.consolidate) ?? Self.nonempty(self.small) ?? Self.nonempty(self.chat),
            smallModel: Self.nonempty(self.small) ?? Self.nonempty(self.chat)
        )
    }

    mutating func selectKind(_ kind: ProviderKindWire) {
        guard kind != self.kind else { return }
        self = Self(kind: kind)
    }

    mutating func selectAuthMethod(_ method: ProviderAuthMethodWire) {
        guard method != self.authMethod, self.kind != .copilot,
              method != .oauth || self.kind.supportsOAuth else { return }
        let kind = self.kind
        self = Self(kind: kind)
        self.authMethod = method
        self.profileID = kind.profileID + (method == .oauth ? "-oauth" : "")
    }

    mutating func selectModel(_ model: ProviderModelInfo) {
        self.chat = model.id
        self.batch = model.id
        self.small = model.id
        self.consolidate = model.id
    }

    mutating func load(_ response: ProvidersResponse, profileID: String? = nil) {
        let selected = profileID ?? response.routes.chat.profile
        guard let profile = response.profiles.first(where: { $0.id == selected }) else { return }
        self = Self(kind: profile.kind)
        self.profileID = profile.id
        self.authMethod = profile.authMethod
        self.baseURL = profile.baseURL ?? ""
        self.organization = profile.organization ?? ""
        self.offline = profile.offline
        self.localRuntime = profile.localRuntime
        self.credentialPresent = profile.credentialPresent
        self.isConfigured = true
        self.chat = profile.models.chat ?? ""
        self.batch = profile.models.goals ?? ""
        self.small = profile.models.small ?? ""
        self.consolidate = profile.models.consolidate ?? ""
        for workload in ProviderWorkload.allCases {
            let route = response.routes.route(for: workload)
            guard route.profile == selected, let model = route.model else { continue }
            switch workload {
            case .chat: self.chat = model
            case .goals: self.batch = model
            case .small: self.small = model
            case .consolidate: self.consolidate = model
            }
        }
    }

    private static func nonempty(_ value: String) -> String? {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }

    private static func validBaseURL(_ value: String) -> Bool {
        guard let components = URLComponents(string: value.trimmingCharacters(in: .whitespacesAndNewlines)),
              let host = components.host, !host.isEmpty,
              components.user == nil, components.password == nil,
              components.query == nil, components.fragment == nil else { return false }
        return components.scheme?.lowercased() == "https"
            || (components.scheme?.lowercased() == "http"
                && ["localhost", "127.0.0.1", "[::1]", "::1"].contains(host.lowercased()))
    }
}
