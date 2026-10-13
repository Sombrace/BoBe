@testable import BoBe
import Foundation
import Testing

@Suite("VoiceCaptureFrameEncoder")
struct VoiceCaptureFrameEncoderTests {
    @Test
    func encodesExactPcmFrameAndClampsSamples() {
        var samples = Array(repeating: Float(0), count: 320)
        samples[0] = 1
        samples[1] = -1
        let frame = VoiceCaptureFrameEncoder.encode(
            streamId: 0x0102_0304,
            sequence: 7,
            samples: samples[...]
        )
        #expect(frame?.count == 653)
        #expect(Array(frame?.prefix(4) ?? Data()) == [1, 2, 3, 4])
        #expect(Array(frame?[13 ..< 17] ?? Data()) == [0xFF, 0x7F, 0x01, 0x80])
    }

    @Test
    func rejectsWrongFrameSizeAndZeroStream() {
        let samples = Array(repeating: Float(0), count: 319)
        #expect(VoiceCaptureFrameEncoder.encode(streamId: 1, sequence: 0, samples: samples[...]) == nil)
        let full = Array(repeating: Float(0), count: 320)
        #expect(VoiceCaptureFrameEncoder.encode(streamId: 0, sequence: 0, samples: full[...]) == nil)
    }
}

@Suite("TtsFrameHeader")
struct TtsFrameHeaderTests {
    private func encode(chunkId: UInt64, flags: UInt8, payload: [UInt8]) -> Data {
        var data = Data(capacity: 9 + payload.count)
        for i in stride(from: 56, through: 0, by: -8) {
            data.append(UInt8((chunkId >> i) & 0xFF))
        }
        data.append(flags)
        data.append(contentsOf: payload)
        return data
    }

    @Test
    func parsesChunkIdBigEndian() {
        let frame = self.encode(chunkId: 0x0102_0304_0506_0708, flags: 0, payload: [0xAA, 0xBB])
        let parsed = TtsFrameHeader.parse(frame)
        #expect(parsed != nil)
        #expect(parsed?.header.chunkId == 0x0102_0304_0506_0708)
    }

    @Test
    func parsesEmptyPayload() {
        let frame = self.encode(chunkId: 42, flags: 0, payload: [])
        let parsed = TtsFrameHeader.parse(frame)
        #expect(parsed?.payload.count == 0)
        #expect(parsed?.header.chunkId == 42)
    }

    @Test
    func parsesPayloadPreservingBytes() {
        let payload: [UInt8] = [0x01, 0x02, 0x03, 0xFF, 0xFE]
        let frame = self.encode(chunkId: 1, flags: 0, payload: payload)
        let parsed = TtsFrameHeader.parse(frame)
        #expect(Array(parsed?.payload ?? Data()) == payload)
    }

    @Test
    func rejectsTooShort() {
        let bad = Data([0, 0, 0, 0, 0, 0, 0, 1])
        #expect(TtsFrameHeader.parse(bad) == nil)
    }

    @Test
    func fillerFlagDetected() {
        let frame = self.encode(chunkId: 1, flags: TtsFrameHeader.flagFiller, payload: [])
        let parsed = TtsFrameHeader.parse(frame)
        #expect(parsed?.header.isFiller == true)
        #expect(parsed?.header.isFirstOfTurn == false)
    }

    @Test
    func firstOfTurnFlagDetected() {
        let frame = self.encode(chunkId: 1, flags: TtsFrameHeader.flagFirstOfTurn, payload: [])
        let parsed = TtsFrameHeader.parse(frame)
        #expect(parsed?.header.isFirstOfTurn == true)
        #expect(parsed?.header.isFiller == false)
    }

    @Test
    func bothFlagsCoexist() {
        let flags = TtsFrameHeader.flagFiller | TtsFrameHeader.flagFirstOfTurn
        let frame = self.encode(chunkId: 1, flags: flags, payload: [])
        let parsed = TtsFrameHeader.parse(frame)
        #expect(parsed?.header.isFiller == true)
        #expect(parsed?.header.isFirstOfTurn == true)
    }
}

@Suite("ServerVoiceMessage")
struct ServerVoiceMessageTests {
    private func decode(_ json: String) throws -> ServerVoiceMessage {
        let data = json.data(using: .utf8) ?? Data()
        return try JSONDecoder().decode(ServerVoiceMessage.self, from: data)
    }

    @Test
    func decodesSpeechPlan() throws {
        let msg = try decode(#"{"type":"speech_plan","plan":{"plan_id":"p1","stt_site":"endpoint","stt_engine_id":"parakeet","tts_site":"daemon","tts_engine_id":"server_pocket","voice_id":"af_bella","uplink":"transcript","downlink":"audio","eou_owner":"endpoint","wake_owner":"endpoint","tts_text_delivery":"none","reason":"voice_v2_endpoint_stt_daemon_tts"}}"#)
        guard case let .speechPlan(plan) = msg else {
            Issue.record("expected .speechPlan, got \(msg)")
            return
        }
        #expect(plan.planId == "p1")
        #expect(plan.sttSite == .endpoint)
        #expect(plan.ttsSite == .daemon)
        #expect(plan.downlink == .audio)
    }

    @Test
    func decodesStateMessage() throws {
        let msg = try decode(#"{"type":"state","phase":"speaking","turn_id":"abc"}"#)
        guard case let .state(phase, turnId) = msg else {
            Issue.record("expected .state, got \(msg)")
            return
        }
        #expect(phase == .speaking)
        #expect(turnId == "abc")
    }

    @Test
    func decodesTruncate() throws {
        let msg = try decode(#"{"type":"truncate","turn_id":"t1","keep_ms":1500}"#)
        guard case let .truncate(turnId, keepMs) = msg else {
            Issue.record("expected .truncate, got \(msg)")
            return
        }
        #expect(turnId == "t1")
        #expect(keepMs == 1500)
    }

    @Test
    func decodesClientTtsText() throws {
        let msg = try decode(
            #"{"type":"tts_text","turn_id":"t1","sequence":2,"text":"Hello."}"#
        )
        guard case let .ttsText(turnId, sequence, text) = msg else {
            Issue.record("expected .ttsText, got \(msg)")
            return
        }
        #expect(turnId == "t1")
        #expect(sequence == 2)
        #expect(text == "Hello.")
    }

    @Test
    func decodesError() throws {
        let msg = try decode(#"{"type":"error","code":"PERMIT_REJECTED","message":"single-flight"}"#)
        guard case let .error(code, message) = msg else {
            Issue.record("expected .error, got \(msg)")
            return
        }
        #expect(code == "PERMIT_REJECTED")
        #expect(message == "single-flight")
    }

    @Test
    func unknownTypeBecomesUnknownVariant() throws {
        let msg = try decode(#"{"type":"future_event","data":42}"#)
        guard case let .unknown(type) = msg else {
            Issue.record("expected .unknown, got \(msg)")
            return
        }
        #expect(type == "future_event")
    }
}

@Suite("ClientVoiceMessage encoding")
struct ClientVoiceMessageEncodingTests {
    private func encode(_ msg: ClientVoiceMessage) throws -> [String: Any] {
        let data = try JSONEncoder().encode(msg)
        let any = try JSONSerialization.jsonObject(with: data)
        return any as? [String: Any] ?? [:]
    }

    private let macOffer = SpeechOfferWire(
        endpointId: UUID(uuidString: "00000000-0000-0000-0000-000000000001") ?? UUID(),
        endpointKind: "mac_app",
        language: "en",
        playbackRates: [24_000],
        endpointStt: [SpeechEngineOfferWire(
            id: "parakeet",
            languages: ["en"],
            partials: true,
            eou: true,
            voices: [],
            sampleRates: [],
            ready: true
        )],
        endpointTts: [],
        audioUplink: false,
        overrides: SpeechOverridesWire(stt: .auto, tts: .daemon),
        daemonIsNonLoopback: false,
        remoteAudioConsent: false
    )

    @Test
    func controlEncodesAbort() throws {
        let obj = try encode(.control(action: .abort))
        #expect(obj["type"] as? String == "control")
        #expect(obj["action"] as? String == "abort")
    }

    @Test(arguments: [VoiceControlAction.mute, .unmute])
    func controlEncodesMuteState(_ action: VoiceControlAction) throws {
        let obj = try encode(.control(action: action))
        #expect(obj["type"] as? String == "control")
        #expect(obj["action"] as? String == action.rawValue)
    }

    @Test
    func transcriptPartialEncodesSnakeCaseKeys() throws {
        let obj = try encode(.transcriptPartial(turnId: "abc", text: "hi"))
        #expect(obj["type"] as? String == "transcript_partial")
        #expect(obj["turn_id"] as? String == "abc")
        #expect(obj["text"] as? String == "hi")
    }

    @Test
    func bargeInCarriesAtomicPartialEvidence() throws {
        let obj = try encode(.bargeIn(
            tsMs: 1_000,
            playbackMsPlayed: 420,
            partialText: "please stop now",
            playheadChunk: 3
        ))
        #expect(obj["type"] as? String == "barge_in")
        #expect(obj["playback_ms_played"] as? Int == 420)
        #expect(obj["partial_text"] as? String == "please stop now")
        #expect(obj["playhead_chunk"] as? Int == 3)
    }

    @Test
    func bargeInOmitsMissingPartialEvidence() throws {
        let obj = try encode(.bargeIn(
            tsMs: 1_000,
            playbackMsPlayed: 420,
            partialText: nil,
            playheadChunk: 0
        ))
        #expect(obj["partial_text"] == nil)
        #expect(obj["playhead_chunk"] as? Int == 0)
    }

    @Test
    func playbackAckEncodesMonotonicPositionFields() throws {
        let obj = try encode(.playbackAck(chunkId: 17, playedMs: 925))
        #expect(obj["type"] as? String == "playback_ack")
        #expect(obj["chunk_id"] as? Int == 17)
        #expect(obj["played_ms"] as? Int == 925)
    }

    @Test
    func helloEncodesVoicePreferencesWhenSet() throws {
        let obj = try encode(.hello(sessionId: "s1", voiceId: "af_alloy", speed: 1.0, speechOffer: self.macOffer))
        #expect(obj["type"] as? String == "hello")
        #expect(obj["session_id"] as? String == "s1")
        #expect(obj["voice_id"] as? String == "af_alloy")
        #expect(obj["speed"] as? Double == 1.0)
        #expect(obj["playback_rate"] == nil)
        #expect(obj["language"] == nil)
    }

    @Test
    func helloOmitsNilOptionals() throws {
        let obj = try encode(.hello(sessionId: "s1", voiceId: nil, speed: nil, speechOffer: self.macOffer))
        #expect(obj["voice_id"] == nil)
        #expect(obj["speed"] == nil)
    }

    @Test
    func helloEncodesCapabilityOffer() throws {
        let obj = try encode(.hello(sessionId: "s1", voiceId: "af_bella", speed: 1, speechOffer: self.macOffer))
        let encodedOffer = obj["speech_offer"] as? [String: Any]
        #expect(encodedOffer?["endpoint_kind"] as? String == "mac_app")
        #expect(encodedOffer?["language"] as? String == "en")
        #expect(encodedOffer?["playback_rates"] as? [Int] == [24_000])
        #expect(encodedOffer?["daemon_is_non_loopback"] as? Bool == false)
        let stt = encodedOffer?["endpoint_stt"] as? [[String: Any]]
        #expect(stt?.first?["id"] as? String == "parakeet")
    }

    @MainActor
    @Test
    func remoteConsentKeysAreAuthorityScoped() {
        let first = UUID(uuidString: "00000000-0000-0000-0000-000000000001") ?? UUID()
        let second = UUID(uuidString: "00000000-0000-0000-0000-000000000002") ?? UUID()
        #expect(RemoteSpeechConsent.storageKey(authorityId: first)
            != RemoteSpeechConsent.storageKey(authorityId: second))
    }

    @MainActor
    @Test
    func supertonicVoiceIdentityReflectsActualStyle() {
        #expect(VoicePipeline.supertonicVoiceId(for: "am_michael") == "supertonic_m1")
        #expect(VoicePipeline.supertonicVoiceId(for: "af_bella") == "supertonic_f1")
    }

    @Test
    func clientTtsCompletionEncodes() throws {
        let obj = try encode(.ttsPlaybackComplete(turnId: "turn-1"))
        #expect(obj["type"] as? String == "tts_playback_complete")
        #expect(obj["turn_id"] as? String == "turn-1")
    }
}
