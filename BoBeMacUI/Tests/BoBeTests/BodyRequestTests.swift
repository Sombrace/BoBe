@testable import BoBe
import Foundation
import Testing

@Suite("Body requests")
struct BodyRequestTests {
    @Test
    func screenTextReadsTopToBottomThenLeftToRight() {
        let text = ScreenReader.arrange([
            .init(text: "footer", top: 0.95, left: 0.1),
            .init(text: "right of title", top: 0.102, left: 0.6),
            .init(text: "title", top: 0.1, left: 0.1),
            .init(text: "   ", top: 0.5, left: 0.1),
            .init(text: "body", top: 0.5, left: 0.2),
        ])
        #expect(text == "title\nright of title\nbody\nfooter")
    }

    @Test
    func screenTextIsBounded() {
        let lines = (0 ..< 2_000).map { index in
            ScreenReader.Line(text: "twelve chars", top: Double(index) / 2_000, left: 0)
        }
        let text = ScreenReader.arrange(lines)
        #expect(text.unicodeScalars.count <= ScreenReader.maxCharacters)
        #expect(text.hasPrefix("twelve chars\ntwelve chars"))
    }

    @Test
    func captureSizeKeepsTheAspectWithinTheLongestSide() {
        #expect(ScreenReader.boundedSize(width: 5_120, height: 2_880) == (3_200, 1_800))
        #expect(ScreenReader.boundedSize(width: 1_512, height: 982) == (1_512, 982))
        #expect(ScreenReader.boundedSize(width: 0, height: 0) == (1, 1))
    }

    @Test
    func requestsArriveAsBodyRequestEventsWithoutACursor() throws {
        let data = Data(#"""
        {"type":"body_request","message_id":"breq_1","payload":{"request_id":"breq_1","action":"screen.read","deadline_ms":20000}}
        """#.utf8)
        let bundle = try JSONDecoder().decode(StreamBundle.self, from: data)
        #expect(bundle.type == .bodyRequest)
        #expect(bundle.eventId == nil)
        let payload = try bundle.payload.decode(as: BodyRequestPayload.self)
        #expect(payload == BodyRequestPayload(requestId: "breq_1", action: BodyRequestWire.screenRead, deadlineMs: 20_000))
    }

    @Test
    func repliesCarryOnlyTheirStatusAndContent() throws {
        let completed = try Self.json(.completed(ScreenReadingWire(text: "inbox", application: "Mail", windowTitle: nil)))
        #expect(completed == ["status": "completed", "result": ["text": "inbox", "application": "Mail"]])
        let declined = try Self.json(.declined("screen_locked"))
        #expect(declined == ["status": "declined", "reason": "screen_locked"])
    }

    @Test
    @MainActor
    func screenNoticeAppearsOnceAndStaysDismissed() {
        let store = BobeStore.shared
        store.screenUnavailableNoticeShown = false
        store.screenUnavailableNotice = false
        defer {
            store.screenUnavailableNoticeShown = false
            store.screenUnavailableNotice = false
        }
        store.noteScreenUnavailable()
        #expect(store.screenUnavailableNotice)
        store.dismissScreenUnavailableNotice()
        store.noteScreenUnavailable()
        #expect(!store.screenUnavailableNotice)
    }

    private static func json(_ reply: BodyReply) throws -> NSDictionary {
        let data = try JSONEncoder().encode(reply)
        return try #require(JSONSerialization.jsonObject(with: data) as? NSDictionary)
    }
}
