import CryptoKit
import Foundation
import Synchronization

nonisolated private final class DaemonSessionCache: Sendable {
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

    private struct State {
        var sessions: [Key: Entry] = [:]
        var nextGeneration: UInt64 = 0
    }

    private let state = Mutex(State())
    private let maximumSessions = 16

    func session(for endpoint: DaemonEndpoint, purpose: DaemonTransportPurpose) -> URLSession {
        let key = Key(
            baseURL: endpoint.baseURL,
            credentialDigest: endpoint.bearerToken.map { Data(SHA256.hash(data: Data($0.utf8))) },
            tlsSPKISha256: endpoint.tlsSPKISha256,
            purpose: purpose
        )

        let maximumSessions = self.maximumSessions
        let (session, evicted) = self.state.withLock { state -> (URLSession, URLSession?) in
            if let cached = state.sessions[key] {
                return (cached.session, nil)
            }
            let session = DaemonTransportPolicy.makeSession(for: endpoint, purpose: purpose)
            state.nextGeneration &+= 1
            state.sessions[key] = Entry(session: session, generation: state.nextGeneration)
            guard state.sessions.count > maximumSessions,
                  let oldest = state.sessions.min(by: { $0.value.generation < $1.value.generation })
            else { return (session, nil) }
            return (session, state.sessions.removeValue(forKey: oldest.key)?.session)
        }
        evicted?.invalidateAndCancel()
        return session
    }

    func invalidateAll() {
        let stale = self.state.withLock { state in
            defer { state.sessions.removeAll(keepingCapacity: true) }
            return state.sessions.values.map(\.session)
        }
        for session in stale {
            session.invalidateAndCancel()
        }
    }
}

nonisolated enum DaemonTransportPurpose: Hashable, Sendable {
    case interactiveHTTP
    case persistentEventStream
    case finiteEventStream
    case attach
    case readinessProbe
    case realtimeWebSocket
    case bodyLoopbackWebSocket
}

nonisolated enum DaemonTransportPolicy {
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

nonisolated private extension DaemonTransportPurpose {
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
