import CryptoKit
import Foundation

private final class DaemonSessionCache: @unchecked Sendable {
    static let shared = DaemonSessionCache()

    private struct Key: Hashable {
        let baseURL: URL
        let credentialDigest: Data?
        let tlsSPKISha256: Data?
        let purpose: DaemonTransportPurpose
    }

    private struct Entry {
        let session: URLSession
        let generation: UInt64
    }

    private let lock = NSLock()
    private var sessions: [Key: Entry] = [:]
    private var nextGeneration: UInt64 = 0
    private let maximumSessions = 16

    func session(for endpoint: DaemonEndpoint, purpose: DaemonTransportPurpose) -> URLSession {
        let key = Key(
            baseURL: endpoint.baseURL,
            credentialDigest: endpoint.bearerToken.map { Data(SHA256.hash(data: Data($0.utf8))) },
            tlsSPKISha256: endpoint.tlsSPKISha256,
            purpose: purpose
        )

        self.lock.lock()
        defer { self.lock.unlock() }
        if let cached = self.sessions[key] {
            return cached.session
        }
        let session = DaemonTransportPolicy.makeSession(for: endpoint, purpose: purpose)
        self.nextGeneration &+= 1
        self.sessions[key] = Entry(session: session, generation: self.nextGeneration)
        if self.sessions.count > self.maximumSessions,
           let oldest = self.sessions.min(by: { $0.value.generation < $1.value.generation }) {
            self.sessions.removeValue(forKey: oldest.key)?.session.invalidateAndCancel()
        }
        return session
    }

    func invalidateAll() {
        self.lock.lock()
        let stale = self.sessions.values.map(\.session)
        self.sessions.removeAll(keepingCapacity: true)
        self.lock.unlock()
        for session in stale {
            session.invalidateAndCancel()
        }
    }
}

enum DaemonTransportPurpose: Hashable, Sendable {
    case interactiveHTTP
    case persistentEventStream
    case finiteEventStream
    case attach
    case readinessProbe
    case realtimeWebSocket
    case bodyLoopbackWebSocket
}

enum DaemonTransportPolicy {
    static func configuration(
        for endpoint: DaemonEndpoint,
        purpose: DaemonTransportPurpose
    ) -> URLSessionConfiguration {
        let configuration = URLSessionConfiguration.default
        let isLoopback = endpoint.baseURL.host.map(DaemonEndpoint.isLoopbackHost) ?? false

        configuration.allowsConstrainedNetworkAccess = true
        configuration.allowsExpensiveNetworkAccess = true
        configuration.waitsForConnectivity = !isLoopback && purpose.waitsForRemoteConnectivity
        configuration.timeoutIntervalForRequest = purpose.requestTimeout
        configuration.timeoutIntervalForResource = purpose.resourceTimeout
        return configuration
    }

    static func session(
        for endpoint: DaemonEndpoint,
        purpose: DaemonTransportPurpose
    ) -> URLSession {
        DaemonSessionCache.shared.session(for: endpoint, purpose: purpose)
    }

    static func invalidateAll() {
        DaemonSessionCache.shared.invalidateAll()
    }

    fileprivate static func makeSession(
        for endpoint: DaemonEndpoint,
        purpose: DaemonTransportPurpose
    ) -> URLSession {
        let configuration = self.configuration(for: endpoint, purpose: purpose)
        guard let pin = endpoint.tlsSPKISha256 else {
            return URLSession(configuration: configuration)
        }
        let delegate = PairingTrustDelegate(expectedSPKI: pin)
        return URLSession(configuration: configuration, delegate: delegate, delegateQueue: nil)
    }
}

private extension DaemonTransportPurpose {
    var waitsForRemoteConnectivity: Bool {
        switch self {
        case .persistentEventStream, .finiteEventStream, .realtimeWebSocket:
            true
        case .interactiveHTTP, .attach, .readinessProbe, .bodyLoopbackWebSocket:
            false
        }
    }

    var requestTimeout: TimeInterval {
        switch self {
        case .interactiveHTTP, .attach:
            10
        case .readinessProbe:
            2
        case .persistentEventStream, .finiteEventStream:
            60
        case .realtimeWebSocket, .bodyLoopbackWebSocket:
            30
        }
    }

    var resourceTimeout: TimeInterval {
        switch self {
        case .interactiveHTTP, .attach, .readinessProbe:
            300
        case .persistentEventStream, .finiteEventStream, .realtimeWebSocket,
             .bodyLoopbackWebSocket:
            7 * 24 * 60 * 60
        }
    }
}
