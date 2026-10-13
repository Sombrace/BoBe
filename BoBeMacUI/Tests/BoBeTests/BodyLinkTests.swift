@testable import BoBe
import Foundation
import Testing

@Suite("BodyLink adapter")
struct BodyLinkTests {
    @Test
    func adapterMediaHeaderParsesTheDaemonRoute() throws {
        let leaseId = UUID()
        var uuid = leaseId.uuid
        var frame = Data([1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7])
        withUnsafeBytes(of: &uuid) { frame.append(contentsOf: $0) }
        frame.append(contentsOf: [0, 0, 0, 42, 0, 0, 0, 3, 0, 0, 0, 0])
        let payload = Data(repeating: 0x5A, count: 660)
        frame.append(payload)

        let parsed = try #require(BodyAdapterMediaHeader.parse(frame))
        #expect(parsed.header == BodyAdapterMediaHeader(
            kind: .microphonePcm,
            connectionGeneration: 7,
            leaseId: leaseId,
            streamId: 42,
            sequence: 3
        ))
        #expect(parsed.payload == payload)

        frame[1] = 2
        #expect(BodyAdapterMediaHeader.parse(frame) == nil)
    }

    @Test
    func microphoneFrameRejectsZeroStreamAndWrongSize() throws {
        var frame = Data(repeating: 0, count: 660)
        frame[0] = 1
        frame[1] = 1
        #expect(BodyLinkMicrophoneFrame.parse(frame) == nil)

        frame[7] = 1
        let parsed = try #require(BodyLinkMicrophoneFrame.parse(frame))
        #expect(parsed.streamId == 1)
        #expect(!parsed.hasDiscontinuity)
        frame[3] = 1
        #expect(BodyLinkMicrophoneFrame.parse(frame)?.hasDiscontinuity == true)
        frame[3] = 2
        #expect(BodyLinkMicrophoneFrame.parse(frame) == nil)
        frame[3] = 0
        #expect(BodyLinkMicrophoneFrame.parse(frame.dropLast()) == nil)
    }

    @Test
    func adapterConfigRequiresExplicitOptInLoopbackAndToken() {
        let valid = BodyAdapterConfig.load(environment: [
            "BOBE_BODY_ADAPTER": "1",
            "BOBE_BODY_ADAPTER_URL": "http://127.0.0.1:8768",
            "BOBE_BODY__ADAPTER_TOKEN": "test-token",
        ])
        #expect(valid?.baseURL.absoluteString == "http://127.0.0.1:8768")

        #expect(BodyAdapterConfig.load(environment: [
            "BOBE_BODY_ADAPTER": "1",
            "BOBE_BODY_ADAPTER_URL": "http://body.example.test:8768",
            "BOBE_BODY__ADAPTER_TOKEN": "test-token",
        ]) == nil)
        #expect(BodyAdapterConfig.load(environment: [
            "BOBE_BODY_ADAPTER": "1",
            "BOBE_BODY_ADAPTER_URL": "http://127.attacker.example:8768",
            "BOBE_BODY__ADAPTER_TOKEN": "test-token",
        ]) == nil)
        #expect(BodyAdapterConfig.load(environment: [
            "BOBE_BODY_ADAPTER": "1",
        ]) == nil)
    }

    @Test
    func pttTranscriptPreservesEveryEouSegmentAndRemainder() {
        #expect(
            combineBodyTranscriptSegments(
                ["first thought", "after a pause"],
                remainder: "final words"
            ) == "first thought after a pause final words"
        )
        #expect(
            combineBodyTranscriptSegments(
                ["same segment"],
                remainder: "same segment"
            ) == "same segment"
        )
    }

    @Test
    func adapterHandshakeCarriesVersionAndCaptureFormat() throws {
        let data = try JSONEncoder().encode(BodyAdapterClientMessage.hello)
        let object = try #require(
            JSONSerialization.jsonObject(with: data) as? [String: Any]
        )

        #expect(object["protocol_major"] as? Int == 1)
        #expect(object["protocol_minor"] as? Int == 1)
        #expect(
            (object["capture_format"] as? [String: Any])?["sample_rate_hz"] as? Int
                == 16_000
        )
    }

    @Test
    func captureOpenDecodesAndRetainsServerFormat() throws {
        let json = """
        {
          "type": "capture.open",
          "route": {
            "device_id": "body-1",
            "connection_generation": 7,
            "body_session_id": "\(UUID().uuidString)",
            "lease_id": "\(UUID().uuidString)",
            "turn_id": "turn-1",
            "capture_stream_id": 11
          },
          "codec": "pcm_s16le",
          "sample_rate_hz": 16000,
          "channels": 1,
          "frame_duration_ms": 20
        }
        """

        let message = try JSONDecoder().decode(
            BodyAdapterServerMessage.self,
            from: Data(json.utf8)
        )
        guard case let .captureOpen(_, format) = message else {
            Issue.record("Expected capture.open")
            return
        }
        #expect(format == .capture)
    }
}
