import Foundation

struct BodyAdapterAudioFormat: Codable, Equatable, Sendable {
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

struct BodyRoute: Codable, Equatable, Sendable {
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

enum BodyAdapterServerMessage: Decodable {
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

enum BodyAdapterClientMessage: Encodable {
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

struct BodyAdapterMediaHeader: Equatable, Sendable {
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
        guard data.count >= self.length,
              data[data.startIndex] == 1,
              let kind = Kind(rawValue: data[data.startIndex + 1]),
              data[data.startIndex + 2] == 0,
              data[data.startIndex + 3] == 0,
              data[data.startIndex + 36 ..< data.startIndex + 40].allSatisfy({ $0 == 0 })
        else {
            return nil
        }
        let generation = readUInt64BE(data, offset: 4)
        let leaseId = readUuid(data, offset: 12)
        let streamId = readUInt32BE(data, offset: 28)
        guard generation != 0, streamId != 0, let leaseId else { return nil }
        return (
            Self(
                kind: kind,
                connectionGeneration: generation,
                leaseId: leaseId,
                streamId: streamId,
                sequence: readUInt32BE(data, offset: 32)
            ),
            data.subdata(in: data.startIndex + self.length ..< data.endIndex)
        )
    }

    private static func readUuid(_ data: Data, offset: Int) -> UUID? {
        let bytes = Array(data.dropFirst(offset).prefix(16))
        guard bytes.count == 16 else { return nil }
        return UUID(uuid: (
            bytes[0], bytes[1], bytes[2], bytes[3],
            bytes[4], bytes[5], bytes[6], bytes[7],
            bytes[8], bytes[9], bytes[10], bytes[11],
            bytes[12], bytes[13], bytes[14], bytes[15]
        ))
    }
}

struct BodyLinkMicrophoneFrame {
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
        guard data.count == self.headerLength + self.payloadLength,
              data[data.startIndex] == 1,
              data[data.startIndex + 1] == 1
        else {
            return nil
        }
        let flags = UInt16(data[data.startIndex + 2]) << 8
            | UInt16(data[data.startIndex + 3])
        guard flags & ~self.discontinuityFlag == 0 else { return nil }
        let streamId = readUInt32BE(data, offset: 4)
        guard streamId != 0 else { return nil }
        return Self(
            flags: flags,
            streamId: streamId,
            sequence: readUInt32BE(data, offset: 8),
            pcm: data.subdata(in: data.startIndex + self.headerLength ..< data.endIndex)
        )
    }
}

private func readUInt32BE(_ data: Data, offset: Int) -> UInt32 {
    data.dropFirst(offset).prefix(4).reduce(0) { ($0 << 8) | UInt32($1) }
}

private func readUInt64BE(_ data: Data, offset: Int) -> UInt64 {
    data.dropFirst(offset).prefix(8).reduce(0) { ($0 << 8) | UInt64($1) }
}
