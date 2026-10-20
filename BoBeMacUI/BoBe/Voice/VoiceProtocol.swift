import Foundation

nonisolated extension VoiceWire {
    static let captureHeaderBytes = 13
    static let capturePayloadBytes = 640
    static let captureSampleRate = 16_000
    static let captureFrameSamples = 320
}

nonisolated enum VoiceControlAction: String, Codable {
    case abort
    case mute
    case unmute
}

nonisolated enum VoicePhaseWire: String, Codable {
    case idle
    case listening
    case thinking
    case speaking
}

nonisolated enum ClientVoiceMessage: Codable {
    case hello(sessionId: String, voiceId: String?, speed: Float?, speechOffer: SpeechOfferWire)
    case bargeIn(tsMs: UInt64, playbackMsPlayed: UInt64, partialText: String?, playheadChunk: UInt64, turnId: String?)
    case playbackAck(chunkId: UInt64, playedMs: UInt64, turnId: String?)
    case control(action: VoiceControlAction)
    case transcriptPartial(turnId: String, text: String)
    case transcriptFinal(turnId: String, text: String)
    case ttsPlaybackStarted(turnId: String, synthesisMs: UInt64)
    case ttsPlaybackComplete(turnId: String)

    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case let .hello(sessionId, voiceId, speed, speechOffer):
            try c.encode("hello", forKey: .type)
            try c.encode(sessionId, forKey: .sessionId)
            try c.encodeIfPresent(voiceId, forKey: .voiceId)
            try c.encodeIfPresent(speed, forKey: .speed)
            try c.encode(speechOffer, forKey: .speechOffer)
        case let .bargeIn(tsMs, playbackMsPlayed, partialText, playheadChunk, turnId):
            try c.encode("barge_in", forKey: .type)
            try c.encode(tsMs, forKey: .tsMs)
            try c.encode(playbackMsPlayed, forKey: .playbackMsPlayed)
            try c.encodeIfPresent(partialText, forKey: .partialText)
            try c.encode(playheadChunk, forKey: .playheadChunk)
            try c.encodeIfPresent(turnId, forKey: .turnId)
        case let .playbackAck(chunkId, playedMs, turnId):
            try c.encode("playback_ack", forKey: .type)
            try c.encode(chunkId, forKey: .chunkId)
            try c.encode(playedMs, forKey: .playedMs)
            try c.encodeIfPresent(turnId, forKey: .turnId)
        case let .control(action):
            try c.encode("control", forKey: .type)
            try c.encode(action, forKey: .action)
        case let .transcriptPartial(turnId, text):
            try c.encode("transcript_partial", forKey: .type)
            try c.encode(turnId, forKey: .turnId)
            try c.encode(text, forKey: .text)
        case let .transcriptFinal(turnId, text):
            try c.encode("transcript_final", forKey: .type)
            try c.encode(turnId, forKey: .turnId)
            try c.encode(text, forKey: .text)
        case let .ttsPlaybackStarted(turnId, synthesisMs):
            try c.encode("tts_playback_started", forKey: .type)
            try c.encode(turnId, forKey: .turnId)
            try c.encode(synthesisMs, forKey: .synthesisMs)
        case let .ttsPlaybackComplete(turnId):
            try c.encode("tts_playback_complete", forKey: .type)
            try c.encode(turnId, forKey: .turnId)
        }
    }

    init(from _: Decoder) throws {
        throw DecodingError.dataCorrupted(
            DecodingError.Context(codingPath: [], debugDescription: "ClientVoiceMessage is encode-only")
        )
    }

    enum CodingKeys: String, CodingKey {
        case type
        case sessionId = "session_id"
        case voiceId = "voice_id"
        case speed
        case speechOffer = "speech_offer"
        case tsMs = "ts_ms"
        case playbackMsPlayed = "playback_ms_played"
        case partialText = "partial_text"
        case playheadChunk = "playhead_chunk"
        case chunkId = "chunk_id"
        case playedMs = "played_ms"
        case action
        case turnId = "turn_id"
        case text
        case synthesisMs = "synthesis_ms"
    }
}

nonisolated enum ServerVoiceMessage: Decodable {
    case speechPlan(SpeechPlanWire)
    case captureCredit(streamId: UInt32, creditFrames: UInt32)
    case sttPartial(turnId: String, text: String)
    case sttFinal(turnId: String, text: String)
    case state(phase: VoicePhaseWire, turnId: String)
    case transcriptFinal(turnId: String, text: String)
    case ttsEnd(turnId: String)
    case ttsText(turnId: String, sequence: UInt64, text: String)
    case truncate(turnId: String, keepMs: UInt64)
    case error(code: String, message: String)
    case unknown(type: String)

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let type = try c.decode(String.self, forKey: .type)
        switch type {
        case "speech_plan":
            self = try .speechPlan(c.decode(SpeechPlanWire.self, forKey: .plan))
        case "capture_credit":
            self = try .captureCredit(
                streamId: c.decode(UInt32.self, forKey: .streamId),
                creditFrames: c.decode(UInt32.self, forKey: .creditFrames)
            )
        case "stt_partial":
            self = try .sttPartial(
                turnId: c.decode(String.self, forKey: .turnId),
                text: c.decode(String.self, forKey: .text)
            )
        case "stt_final":
            self = try .sttFinal(
                turnId: c.decode(String.self, forKey: .turnId),
                text: c.decode(String.self, forKey: .text)
            )
        case "state":
            self = try .state(
                phase: c.decode(VoicePhaseWire.self, forKey: .phase),
                turnId: c.decode(String.self, forKey: .turnId)
            )
        case "transcript_final":
            self = try .transcriptFinal(
                turnId: c.decode(String.self, forKey: .turnId),
                text: c.decode(String.self, forKey: .text)
            )
        case "tts_end":
            self = try .ttsEnd(turnId: c.decode(String.self, forKey: .turnId))
        case "tts_text":
            self = try .ttsText(
                turnId: c.decode(String.self, forKey: .turnId),
                sequence: c.decode(UInt64.self, forKey: .sequence),
                text: c.decode(String.self, forKey: .text)
            )
        case "truncate":
            self = try .truncate(
                turnId: c.decode(String.self, forKey: .turnId),
                keepMs: c.decode(UInt64.self, forKey: .keepMs)
            )
        case "error":
            self = try .error(
                code: c.decode(String.self, forKey: .code),
                message: c.decode(String.self, forKey: .message)
            )
        default:
            self = .unknown(type: type)
        }
    }

    enum CodingKeys: String, CodingKey {
        case type
        case phase
        case turnId = "turn_id"
        case text
        case keepMs = "keep_ms"
        case code
        case message
        case sequence
        case plan
        case streamId = "stream_id"
        case creditFrames = "credit_frames"
    }
}

nonisolated struct VoiceCaptureFrameEncoder {
    static func encode(streamId: UInt32, sequence: UInt32, samples: ArraySlice<Float>) -> Data? {
        guard streamId != 0, samples.count == VoiceWire.captureFrameSamples else { return nil }
        var data = Data(count: VoiceWire.captureHeaderBytes + VoiceWire.capturePayloadBytes)
        var bytes = data.mutableBytes
        bytes.storeBytes(of: streamId.bigEndian, toByteOffset: 0, as: UInt32.self)
        bytes.storeBytes(of: sequence.bigEndian, toByteOffset: 4, as: UInt32.self)
        for (index, sample) in samples.enumerated() {
            let pcm = Int16(clamping: Int32((max(-1, min(1, sample)) * 32_767).rounded()))
            bytes.storeBytes(
                of: pcm.littleEndian,
                toByteOffset: VoiceWire.captureHeaderBytes + index * MemoryLayout<Int16>.size,
                as: Int16.self
            )
        }
        return data
    }
}

nonisolated struct TtsFrameHeader {
    let chunkId: UInt64
    let flags: UInt8

    static let length = 9
    static let flagFiller: UInt8 = 0b0000_0001
    static let flagFirstOfTurn: UInt8 = 0b0000_0010

    static func parse(_ data: Data) -> (header: TtsFrameHeader, payload: Data)? {
        let bytes = data.bytes
        guard bytes.byteCount >= self.length else { return nil }
        let header = TtsFrameHeader(
            chunkId: UInt64(bigEndian: bytes.load(fromByteOffset: 0, as: UInt64.self)),
            flags: bytes.load(fromByteOffset: 8, as: UInt8.self)
        )
        return (header, data.dropFirst(self.length))
    }

    var isFiller: Bool {
        self.flags & Self.flagFiller != 0
    }

    var isFirstOfTurn: Bool {
        self.flags & Self.flagFirstOfTurn != 0
    }
}
