@testable import BoBe
import Foundation
import SwiftUI
import Testing

@MainActor
@Suite("iPhone conversation")
struct ConversationTests {
    @Test
    func retryResendsTheSameRequestIdentity() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        try await harness.connect()
        link.fail("POST /message")
        harness.model.send("Remember the plan")
        try await eventually { harness.model.thread.last?.delivery == .failed }

        link.reply("POST /message", Fixture.accepted("msg_00000000000000000000000000000aaa", replayed: true))
        harness.model.retry(try #require(harness.model.thread.last))
        try await eventually { harness.model.thread.last?.delivery == .sent }

        let identities = link.requests("POST /message").map { $0.json["request_id"] as? String }
        #expect(identities.count == 2)
        #expect(identities[0] != nil)
        #expect(identities[0] == identities[1])
        #expect(harness.model.thread.last?.replyId == "msg_00000000000000000000000000000aaa")
    }

    @Test
    func unsafeReplayAsksBeforeSendingAgain() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        try await harness.connect()
        link.fail("POST /message", .http(409, code: "REQUEST_REPLAY_UNSAFE"))
        harness.model.send("Book the table")
        try await eventually { harness.model.thread.last?.delivery == .unconfirmed }

        let message = try #require(harness.model.thread.last)
        harness.model.retry(message)
        await pause()
        #expect(link.requests("POST /message").count == 1)

        link.reply("POST /message", Fixture.accepted("msg_00000000000000000000000000000bbb"))
        harness.model.sendAgain(message)
        try await eventually { harness.model.thread.last?.delivery == .sent }
        let identities = link.requests("POST /message").compactMap { $0.json["request_id"] as? String }
        #expect(identities.count == 2)
        #expect(identities[0] != identities[1])
    }

    @Test
    func acceptedMessagesStayUntilTheConversationHasThem() {
        var local = ThreadMessage.outgoing("Plan my week")
        local.delivery = .sent
        local.replyId = "msg_reply"
        let user = ThreadMessage(id: "msg_user", author: .me, text: "Plan my week", isComplete: true, delivery: .sent, date: .now)
        let reply = ThreadMessage(id: "msg_reply", author: .bobe, text: "Sure", isComplete: false, delivery: .sent, date: .now)

        #expect(ThreadMessage.merge(canonical: [], local: [local]).map(\.id) == [local.id])
        #expect(ThreadMessage.merge(canonical: [user], local: [local]).map(\.id) == ["msg_user"])
        #expect(ThreadMessage.merge(canonical: [reply], local: [local]).map(\.id) == ["msg_reply"])

        var failed = local
        failed.delivery = .failed
        failed.replyId = nil
        #expect(ThreadMessage.merge(canonical: [user], local: [failed]).map(\.id) == ["msg_user", failed.id])
    }

    @Test
    func triggerWarningsAreNotFailedReplies() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        try await harness.connect()
        link.emit(try Fixture.event(
            "error",
            messageId: UUID().uuidString,
            payload: ["trigger": "vision", "message": "Screen awareness is paused.", "recoverable": true]
        ))
        link.emit(try Fixture.event("indicator", payload: ["indicator": "THINKING"]))
        try await eventually { harness.model.indicator == .thinking }
        #expect(!harness.model.replyTrouble)
    }

    @Test
    func failedRepliesSettleTheirPartialMessage() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        try await harness.connect()
        link.emit(try Fixture.event("text_delta", messageId: "msg_r", payload: ["delta": "Half a th", "done": false]))
        try await eventually { harness.model.thread.last?.isStreaming == true }

        link.emit(try Fixture.event(
            "error",
            messageId: "msg_r",
            payload: ["code": "CHAT_STREAM_CLOSED", "message": "closed", "recoverable": true]
        ))
        try await eventually { harness.model.thread.last?.isStreaming == false }
        #expect(harness.model.replyTrouble)
        #expect(harness.model.thread.last?.text == "Half a th")
        #expect(harness.model.thread.last?.isComplete == false)
    }

    @Test
    func cancelledTurnsSettleQuietly() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        try await harness.connect()
        link.emit(try Fixture.event("text_delta", messageId: "msg_c", payload: ["delta": "Let me", "done": false]))
        link.emit(try Fixture.event(
            "error",
            messageId: "msg_c",
            payload: ["code": "turn_cancelled", "message": "stopped", "recoverable": true]
        ))
        try await eventually { harness.model.thread.last?.isStreaming == false }
        #expect(!harness.model.replyTrouble)
    }

    @Test
    func parallelToolsKeepBoBeWorking() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        try await harness.connect()
        link.emit(try Fixture.event("tool_call_start", payload: ["status": "start", "tool_name": "web", "tool_call_id": "a"]))
        link.emit(try Fixture.event("tool_call_start", payload: ["status": "start", "tool_name": "files", "tool_call_id": "b"]))
        link.emit(try Fixture.event(
            "tool_call_complete",
            payload: ["status": "complete", "tool_name": "web", "tool_call_id": "a", "success": true]
        ))
        try await eventually { harness.model.activeTools == ["b"] }
        #expect(harness.model.mood == .working)
        #expect(harness.model.status == L10n.tr("phone.status.working"))

        link.emit(try Fixture.event("indicator", payload: ["indicator": "IDLE"]))
        try await eventually { harness.model.activeTools.isEmpty }
    }

    @Test
    func disconnectingForgetsRunningTools() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        try await harness.connect()
        link.emit(try Fixture.event("tool_call_start", payload: ["status": "start", "tool_name": "web", "tool_call_id": "a"]))
        try await eventually { !harness.model.activeTools.isEmpty }
        harness.model.scenePhaseChanged(.background)
        #expect(harness.model.activeTools.isEmpty)
    }

    @Test
    func failedSnapshotRecoveryKeepsTheCursor() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        link.stream(StreamScript(items: [
            .connected,
            try Fixture.event("indicator", eventId: 5, payload: ["indicator": "IDLE"]),
            try Fixture.gap(resumeAfter: 9),
        ], ends: true))
        link.script("GET /conversation/current", [
            .success(Fixture.snapshot()), .failure(.unreachable), .success(Fixture.snapshot()),
        ])
        harness.model.start()
        try await eventually(.seconds(20)) { link.cursors.count >= 2 }
        #expect(link.cursors[1] == EventCursor(eventId: 5, incarnation: Fixture.incarnation))
        try await eventually { harness.model.link == .online }
    }

    @Test
    func recoveredSnapshotResumesAfterTheGap() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        link.stream(StreamScript(items: [
            .connected,
            try Fixture.event("indicator", eventId: 5, payload: ["indicator": "IDLE"]),
            try Fixture.gap(resumeAfter: 9),
        ], ends: true))
        harness.model.start()
        try await eventually { link.cursors.count >= 2 }
        #expect(link.cursors[1] == EventCursor(eventId: 9, incarnation: Fixture.incarnation))
    }

    @Test(arguments: ["conversation_changed", "conversation_closed"])
    func failedConversationRefreshKeepsTheCursor(event: String) async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        link.stream(StreamScript(items: [
            .connected,
            try Fixture.event("indicator", eventId: 5, payload: ["indicator": "IDLE"]),
            try Fixture.conversationEvent(event, eventId: 6),
        ], ends: true))
        let current = "msg_00000000000000000000000000000ccc"
        link.script("GET /conversation/current", [
            .success(Fixture.snapshot()),
            .failure(.http(503, code: "SERVICE_UNAVAILABLE")),
            .success(Fixture.snapshot([Fixture.turn(current, role: "assistant", content: "Fresh start")])),
        ])
        harness.model.start()
        try await eventually(.seconds(20)) { link.cursors.count >= 2 }
        #expect(link.cursors[1] == EventCursor(eventId: 5, incarnation: Fixture.incarnation))
        try await eventually { harness.model.link == .online && harness.model.thread.map(\.id) == [current] }
    }

    @Test(arguments: ["conversation_changed", "conversation_closed"])
    func refreshedConversationAdvancesTheCursor(event: String) async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        link.stream(StreamScript(items: [
            .connected,
            try Fixture.event("indicator", eventId: 5, payload: ["indicator": "IDLE"]),
            try Fixture.conversationEvent(event, eventId: 6),
        ], ends: true))
        harness.model.start()
        try await eventually { link.cursors.count >= 2 }
        #expect(link.cursors[1] == EventCursor(eventId: 6, incarnation: Fixture.incarnation))
    }
}
