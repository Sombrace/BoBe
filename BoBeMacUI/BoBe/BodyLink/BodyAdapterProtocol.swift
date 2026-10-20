import Foundation

nonisolated struct BodyAdapterAudioFormat: Codable, Equatable, Sendable {
    let codec: String
    let sampleRateHz: UInt32
    let channels: UInt8
    let frameDurationMs: UInt16

    static let capture = Self(
        codec: "pcm_s16le",
        sampleRateHz: 16_000,
        channels: 1,
        frameDurationMs: 20
    )

    enum CodingKeys: String, CodingKey {
        case codec
        case sampleRateHz = "sample_rate_hz"
        case channels
        case frameDurationMs = "frame_duration_ms"
    }
}

nonisolated struct BodyRoute: Codable, Equatable, Sendable {
    let deviceId: String
    let connectionGeneration: UInt64
    let bodySessionId: UUID
    let leaseId: UUID
    let turnId: String
    let captureStreamId: UInt32

    enum CodingKeys: String, CodingKey {
        case deviceId = "device_id"
        case connectionGeneration = "connection_generation"
        case bodySessionId = "body_session_id"
        case leaseId = "lease_id"
        case turnId = "turn_id"
        case captureStreamId = "capture_stream_id"
    }
}

nonisolated enum BodyAdapterServerMessage: Decodable {
    case welcome(protocolMajor: UInt8, protocolMinor: UInt8, adapterGeneration: UInt64)
    case captureOpen(route: BodyRoute, format: BodyAdapterAudioFormat)
    case captureClose(route: BodyRoute)
    case turnCancel(route: BodyRoute, reason: String)
    case turnComplete(route: BodyRoute, reason: String)
    case unknown

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .type) {
        case "adapter.welcome":
            self = try .welcome(
                protocolMajor: container.decode(UInt8.self, forKey: .protocolMajor),
                protocolMinor: container.decode(UInt8.self, forKey: .protocolMinor),
                adapterGeneration: container.decode(UInt64.self, forKey: .adapterGeneration)
            )
        case "capture.open":
            self = try .captureOpen(
                route: container.decode(BodyRoute.self, forKey: .route),
                format: BodyAdapterAudioFormat(
                    codec: container.decode(String.self, forKey: .codec),
                    sampleRateHz: container.decode(UInt32.self, forKey: .sampleRateHz),
                    channels: container.decode(UInt8.self, forKey: .channels),
                    frameDurationMs: container.decode(UInt16.self, forKey: .frameDurationMs)
                )
            )
        case "capture.close":
            self = try .captureClose(route: container.decode(BodyRoute.self, forKey: .route))
        case "turn.cancel":
            self = try .turnCancel(
                route: container.decode(BodyRoute.self, forKey: .route),
                reason: container.decode(String.self, forKey: .reason)
            )
        case "turn.complete":
            self = try .turnComplete(
                route: container.decode(BodyRoute.self, forKey: .route),
                reason: container.decode(String.self, forKey: .reason)
            )
        default:
            self = .unknown
        }
    }

    private enum CodingKeys: String, CodingKey {
        case type
        case protocolMajor = "protocol_major"
        case protocolMinor = "protocol_minor"
        case adapterGeneration = "adapter_generation"
        case route
        case reason
        case codec
        case sampleRateHz = "sample_rate_hz"
        case channels
        case frameDurationMs = "frame_duration_ms"
    }
}

nonisolated enum BodyAdapterClientMessage: Encodable {
    case hello
    case transcriptPartial(route: BodyRoute, text: String)
    case transcriptFinal(route: BodyRoute, text: String)
    case transcriptEmpty(route: BodyRoute)

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .hello:
            try container.encode("adapter.hello", forKey: .type)
            try container.encode(1, forKey: .protocolMajor)
            try container.encode(1, forKey: .protocolMinor)
            try container.encode(BodyAdapterAudioFormat.capture, forKey: .captureFormat)
        case let .transcriptPartial(route, text):
            try container.encode("transcript.partial", forKey: .type)
            try container.encode(route.connectionGeneration, forKey: .connectionGeneration)
            try container.encode(route.leaseId, forKey: .leaseId)
            try container.encode(text, forKey: .text)
        case let .transcriptFinal(route, text):
            try container.encode("transcript.final", forKey: .type)
            try container.encode(route.connectionGeneration, forKey: .connectionGeneration)
            try container.encode(route.leaseId, forKey: .leaseId)
            try container.encode(text, forKey: .text)
        case let .transcriptEmpty(route):
            try container.encode("transcript.empty", forKey: .type)
            try container.encode(route.connectionGeneration, forKey: .connectionGeneration)
            try container.encode(route.leaseId, forKey: .leaseId)
        }
    }

    private enum CodingKeys: String, CodingKey {
        case type
        case protocolMajor = "protocol_major"
        case protocolMinor = "protocol_minor"
        case captureFormat = "capture_format"
        case connectionGeneration = "connection_generation"
        case leaseId = "lease_id"
        case text
    }
}

nonisolated struct BodyAdapterMediaHeader: Equatable, Sendable {
    enum Kind: UInt8 {
        case microphonePcm = 1
    }

    static let length = 40
    let kind: Kind
    let connectionGeneration: UInt64
    let leaseId: UUID
    let streamId: UInt32
    let sequence: UInt32

    static func parse(_ data: Data) -> (header: Self, payload: Data)? {
        let bytes = data.bytes
        guard bytes.byteCount >= self.length,
              bytes.load(fromByteOffset: 0, as: UInt8.self) == 1,
              let kind = Kind(rawValue: bytes.load(fromByteOffset: 1, as: UInt8.self)),
              bytes.load(fromByteOffset: 2, as: UInt16.self) == 0,
              bytes.load(fromByteOffset: 36, as: UInt32.self) == 0
        else {
            return nil
        }
        let generation = UInt64(bigEndian: bytes.load(fromByteOffset: 4, as: UInt64.self))
        let streamId = UInt32(bigEndian: bytes.load(fromByteOffset: 28, as: UInt32.self))
        guard generation != 0, streamId != 0 else { return nil }
        return (
            Self(
                kind: kind,
                connectionGeneration: generation,
                leaseId: UUID(uuid: bytes.unsafeLoadUnaligned(fromByteOffset: 12, as: uuid_t.self)),
                streamId: streamId,
                sequence: UInt32(bigEndian: bytes.load(fromByteOffset: 32, as: UInt32.self))
            ),
            data.dropFirst(self.length)
        )
    }
}

nonisolated struct BodyLinkMicrophoneFrame {
    static let headerLength = 20
    static let payloadLength = 640
    static let discontinuityFlag: UInt16 = 0x0001
    let flags: UInt16
    let streamId: UInt32
    let sequence: UInt32
    let pcm: Data

    var hasDiscontinuity: Bool {
        self.flags & Self.discontinuityFlag != 0
    }

    static func parse(_ data: Data) -> Self? {
        let bytes = data.bytes
        guard bytes.byteCount == self.headerLength + self.payloadLength,
              bytes.load(fromByteOffset: 0, as: UInt8.self) == 1,
              bytes.load(fromByteOffset: 1, as: UInt8.self) == 1
        else {
            return nil
        }
        let flags = UInt16(bigEndian: bytes.load(fromByteOffset: 2, as: UInt16.self))
        guard flags & ~self.discontinuityFlag == 0 else { return nil }
        let streamId = UInt32(bigEndian: bytes.load(fromByteOffset: 4, as: UInt32.self))
        guard streamId != 0 else { return nil }
        return Self(
            flags: flags,
            streamId: streamId,
            sequence: UInt32(bigEndian: bytes.load(fromByteOffset: 8, as: UInt32.self)),
            pcm: data.dropFirst(self.headerLength)
        )
    }
}
