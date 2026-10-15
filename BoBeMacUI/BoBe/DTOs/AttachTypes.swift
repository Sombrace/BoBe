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
    let stt: CapabilityStateWire
}

struct HelloReadiness: Codable, Sendable {
    let state: String
    let blockingReasons: [String]

    enum CodingKeys: String, CodingKey {
        case state
        case blockingReasons = "blocking_reasons"
    }
}

struct HelloResponse: Codable, Sendable {
    let runtimeAuthorityId: UUID
    let runtimeAuthorityEpoch: Int
    let daemonVersion: String
    let negotiatedProtocol: UInt32
    let deploymentMode: String
    let initializationState: InitializationStateWire
    let readiness: HelloReadiness
    let capabilities: RuntimeCapabilitiesWire
    let grantedScopes: [String]

    enum CodingKeys: String, CodingKey {
        case runtimeAuthorityId = "runtime_authority_id"
        case runtimeAuthorityEpoch = "runtime_authority_epoch"
        case daemonVersion = "daemon_version"
        case negotiatedProtocol = "negotiated_protocol"
        case deploymentMode = "deployment_mode"
        case initializationState = "initialization_state"
        case readiness, capabilities
        case grantedScopes = "granted_scopes"
    }
}
