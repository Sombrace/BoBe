import Foundation

enum AttachWire {
    static let protocolMin: UInt32 = 1
    static let protocolMax: UInt32 = 1
    static let path = "/v1/hello"
}

struct HelloRequest: Codable, Sendable {
    let surfaceId: UUID
    let surfaceKind: String
    let surfaceAppVersion: String
    let protocolMin: UInt32
    let protocolMax: UInt32
    let capabilities: SurfaceCapabilitiesWire
    let requestedScopes: [String]

    enum CodingKeys: String, CodingKey {
        case surfaceId = "surface_id"
        case surfaceKind = "surface_kind"
        case surfaceAppVersion = "surface_app_version"
        case protocolMin = "protocol_min"
        case protocolMax = "protocol_max"
        case capabilities
        case requestedScopes = "requested_scopes"
    }
}

struct SurfaceCapabilitiesWire: Codable, Sendable {
    let microphone: Bool
    let playback: Bool
    let wake: Bool
    let localStt: Bool
    let localTts: Bool
    let screenContext: Bool

    enum CodingKeys: String, CodingKey {
        case microphone, playback, wake
        case localStt = "local_stt"
        case localTts = "local_tts"
        case screenContext = "screen_context"
    }
}

enum CapabilityStateWire: String, Codable, Sendable {
    case ready, degraded, unsupported
}

struct RuntimeCapabilitiesWire: Codable, Sendable {
    let agent: CapabilityStateWire
    let stt: CapabilityStateWire
    let tts: CapabilityStateWire
    let capture: CapabilityStateWire
    let body: CapabilityStateWire
    let mcp: CapabilityStateWire
    let events: CapabilityStateWire
}

struct HelloReadiness: Codable, Sendable {
    let state: String
    let blockingReasons: [String]

    enum CodingKeys: String, CodingKey {
        case state
        case blockingReasons = "blocking_reasons"
    }
}

struct EventsContractWire: Codable, Sendable {
    let subscription: String
    let cursorStart: UInt64?

    enum CodingKeys: String, CodingKey {
        case subscription
        case cursorStart = "cursor_start"
    }
}

struct HelloResponse: Codable, Sendable {
    let runtimeAuthorityId: UUID
    let runtimeAuthorityEpoch: Int
    let runtimeIncarnationId: UUID
    let daemonVersion: String
    let schemaVersion: Int
    let startedAt: String
    let protocolMin: UInt32
    let protocolMax: UInt32
    let negotiatedProtocol: UInt32
    let deploymentMode: String
    let initializationState: InitializationStateWire
    let readiness: HelloReadiness
    let capabilities: RuntimeCapabilitiesWire
    let grantedScopes: [String]
    let authMode: String
    let events: EventsContractWire

    enum CodingKeys: String, CodingKey {
        case runtimeAuthorityId = "runtime_authority_id"
        case runtimeAuthorityEpoch = "runtime_authority_epoch"
        case runtimeIncarnationId = "runtime_incarnation_id"
        case daemonVersion = "daemon_version"
        case schemaVersion = "schema_version"
        case startedAt = "started_at"
        case protocolMin = "protocol_min"
        case protocolMax = "protocol_max"
        case negotiatedProtocol = "negotiated_protocol"
        case deploymentMode = "deployment_mode"
        case initializationState = "initialization_state"
        case readiness, capabilities
        case grantedScopes = "granted_scopes"
        case authMode = "auth_mode"
        case events
    }
}
