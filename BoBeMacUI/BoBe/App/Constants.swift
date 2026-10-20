import Darwin
import Foundation

nonisolated enum DaemonEndpoint: Sendable, Equatable {
    case unavailable
    case managedLocal(bearerToken: String?)
    case attachedLocal(baseURL: URL, bearerToken: String?)
    case remote(baseURL: URL, bearerToken: String, tlsSPKISha256: Data? = nil)

    var baseURL: URL {
        switch self {
        case .unavailable:
            return URL(fileURLWithPath: "/dev/null")
        case .managedLocal:
            return URL(string: "http://127.0.0.1:\(DaemonConfig.managedPort)")
                ?? URL(fileURLWithPath: "/")
        case let .attachedLocal(baseURL, _), let .remote(baseURL, _, _):
            return baseURL
        }
    }

    var bearerToken: String? {
        switch self {
        case .unavailable:
            return nil
        case let .managedLocal(token): return token
        case let .attachedLocal(_, token): return token
        case let .remote(_, token, _): return token
        }
    }

    var tlsSPKISha256: Data? {
        if case let .remote(_, _, pin) = self { return pin }
        return nil
    }

    var managesLocalProcess: Bool {
        if case .managedLocal = self { return true }
        return false
    }

    var canRestartRuntime: Bool { self.managesLocalProcess }

    var isLoopbackRuntime: Bool {
        guard let host = self.baseURL.host else { return false }
        return Self.isLoopbackHost(host)
    }

    var daemonFilesystemIsLocalToApp: Bool {
        if case .managedLocal = self { return true }
        return false
    }

    func authorize(_ request: inout URLRequest) {
        guard let token = self.bearerToken else { return }
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
    }

    static func isLoopbackHost(_ host: String) -> Bool {
        let normalized = host.lowercased()
        if normalized == "localhost" || normalized == "::1" {
            return true
        }
        var address = in_addr()
        let parsed = normalized.withCString {
            inet_pton(AF_INET, $0, &address)
        }
        guard parsed == 1 else { return false }
        return withUnsafeBytes(of: &address) { bytes in
            bytes.first == 127
        }
    }
}

nonisolated enum DaemonConfig {
    static let defaultHost = "127.0.0.1"
    static let defaultPort = 8766

    static var managedPort: Int {
        self.managedPort(environment: ProcessInfo.processInfo.environment)
    }

    static func managedPort(environment: [String: String]) -> Int {
        guard environment["BOBE_DEV"] == "1",
              let raw = environment["BOBE_MANAGED_PORT"],
              let port = Int(raw),
              (1 ... Int(UInt16.max)).contains(port)
        else { return self.defaultPort }
        return port
    }

    static var managedDataDirectory: URL {
        self.managedDataDirectory(
            environment: ProcessInfo.processInfo.environment,
            homeDirectory: FileManager.default.homeDirectoryForCurrentUser
        )
    }

    static func managedDataDirectory(
        environment: [String: String],
        homeDirectory: URL
    ) -> URL {
        if environment["BOBE_DEV"] == "1",
           let path = environment["BOBE_MANAGED_DATA_DIR"],
           path.hasPrefix("/") {
            return URL(fileURLWithPath: path, isDirectory: true).standardizedFileURL
        }
        return homeDirectory.appendingPathComponent(BobePaths.dataDirName, isDirectory: true)
    }

    static var endpoint: DaemonEndpoint { EndpointStore.shared.endpoint ?? .unavailable }
    static var baseURL: String { self.endpoint.baseURL.absoluteString }
}

nonisolated enum PrivacyWire {
    static let purgeRequestTimeoutSeconds: TimeInterval = 60
}

nonisolated struct BodyAdapterConfiguration: Sendable {
    let baseURL: URL
    let bearerToken: String
}

nonisolated enum BodyAdapterConfig {
    static func load(
        environment: [String: String] = ProcessInfo.processInfo.environment
    ) -> BodyAdapterConfiguration? {
        guard environment["BOBE_BODY_ADAPTER"] == "1" else { return nil }
        let rawURL = environment["BOBE_BODY_ADAPTER_URL"] ?? "http://127.0.0.1:8768"
        guard let baseURL = URL(string: rawURL),
              baseURL.scheme?.lowercased() == "http",
              baseURL.host.map(DaemonEndpoint.isLoopbackHost) == true,
              baseURL.user == nil,
              baseURL.password == nil,
              baseURL.query == nil,
              baseURL.fragment == nil,
              let token = environment["BOBE_BODY__ADAPTER_TOKEN"],
              !token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            return nil
        }
        return BodyAdapterConfiguration(baseURL: baseURL, bearerToken: token)
    }
}

nonisolated enum OllamaDefaults {
    static let baseURL = "http://127.0.0.1:11434"
    static let v1URL = "http://127.0.0.1:11434/v1"
}

nonisolated enum BobePaths {
    static let dataDirName = ".bobe"
}

nonisolated enum ToolCallStatusWire {
    static let start = "start"
    static let complete = "complete"
}

nonisolated enum PauseSensitivityMs {
    static let tight: Int = 400
    static let balanced: Int = 800
    static let patient: Int = 1500
}

nonisolated enum VoiceEngineId {
    static let appleSpeechAnalyzer = "apple_speech_analyzer"
    static let appleSpeechSynthesizer = "apple_speech_synthesizer"
    static let serverPocket = "server_pocket"
}

nonisolated enum InstallStatusWire: String, Codable, Sendable, Equatable {
    case idle
    case running
    case complete
    case canceled
    case failed
}

nonisolated enum VoiceWire {
    static let ttsOutputSampleRate = 24_000
    static let pocketTtsModelDir = "pocket-tts-en-b6369a24-q8"
    static let defaultPersona = "alba"
    static let modelKindTts = "tts"
    static let subprotocolV2 = "bobe.voice.v2"
}

nonisolated enum McpServerStatusWire {
    static let connected = "connected"
    static let failed = "failed"
    static let pending = "pending"
    static let stopped = "stopped"
}

nonisolated enum WindowSizes {
    static let margin: CGFloat = 16
    static let shadowInset: CGFloat = 26
    static let clusterTrailing: CGFloat = 18
    static let clusterBottom: CGFloat = 18
    static let columnAmbient: CGFloat = 304
    static let columnAsk: CGFloat = 364
    static let columnConversation: CGFloat = 428
    static let columnBaseline: CGFloat = 37
    static let heightCollapsed: CGFloat = 164
    static let heightAmbient: CGFloat = 430
    static let heightAsk: CGFloat = 620
    static let heightConversation: CGFloat = 760
    static let heightThreadChrome: CGFloat = 115
    static let heightChatViewportMin: CGFloat = 48
    static let heightChatViewportMax: CGFloat = 560
    static let heightAmbientMessageViewport: CGFloat = 240
    static let heightMax: CGFloat = 900
    static let settleMilliseconds = 560
}

nonisolated enum StoreTiming {
    static let textDeltaFlushMilliseconds = 150
    static let lastMessageClearSeconds: TimeInterval = 30
    static let toolCompletionLingerSeconds: TimeInterval = 5
    static let conversationClearSeconds: TimeInterval = 3
    static let reconnectStatusDelayMilliseconds = 1500
    static let screenGlanceMilliseconds = 1200
}

nonisolated enum InactivityTiming {
    static let timeoutSeconds: TimeInterval = 10 * 60
    static let settleSeconds: TimeInterval = 90
    static let checkIntervalSeconds: TimeInterval = 30
}
