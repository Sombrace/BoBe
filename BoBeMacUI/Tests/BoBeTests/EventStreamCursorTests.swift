@testable import BoBe
import Foundation
import Testing

@Suite("Event stream cursor and recovery", .serialized)
struct EventStreamCursorTests {
    private static let incarnation = UUID(uuidString: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")!
    private static let selection = EndpointSelection(
        generation: 1, profileID: UUID(), authorityID: UUID(), endpoint: .unavailable
    )

    @Test
    @MainActor
    func staleConversationChangeIsConsumedAndTheNewestSnapshotIsKept() async throws {
        let store = BobeStore(
            currentSelection: { Self.selection },
            conversationSnapshot: { _ in Self.snapshot("Reply in conversation B") }
        )
        let client = DaemonClient(endpoint: .unavailable)

        try await client.receiveEvents(Self.lines([
            Self.event(10, "conversation_changed", #"{"conversation_id":"conversation-a"}"#),
            Self.event(11, "conversation_changed", #"{"conversation_id":"conversation-b"}"#),
        ]), handler: { await store.processBundle($0) })

        #expect(await client.lastProcessedEventId == 11)
        #expect(store.messages.map(\.content) == ["Reply in conversation B"])
    }

    @Test
    @MainActor
    func conversationChangeStaysUnacknowledgedWhenTheSnapshotCannotLoad() async throws {
        let store = BobeStore(
            currentSelection: { Self.selection },
            conversationSnapshot: { _ in throw URLError(.cannotConnectToHost) }
        )
        let client = DaemonClient(endpoint: .unavailable)

        try await client.receiveEvents(Self.lines([
            Self.event(9, "indicator", #"{"indicator":"THINKING"}"#),
            Self.event(10, "conversation_changed", #"{"conversation_id":"conversation-b"}"#),
            Self.event(11, "indicator", #"{"indicator":"IDLE"}"#),
        ]), handler: { await store.processBundle($0) })

        #expect(await client.lastProcessedEventId == 9)
        #expect(store.isThinking)
    }

    @Test
    @MainActor
    func malformedKnownPayloadDoesNotAdvanceTheCursor() async throws {
        let store = BobeStore(currentSelection: { Self.selection })
        let client = DaemonClient(endpoint: .unavailable)

        try await client.receiveEvents(Self.lines([
            Self.event(1, "indicator", #"{"indicator":"THINKING"}"#),
            Self.event(2, "text_delta", #"{"delta":7,"done":"yes"}"#),
            Self.event(3, "indicator", #"{"indicator":"IDLE"}"#),
        ]), handler: { await store.processBundle($0) })

        #expect(await client.lastProcessedEventId == 1)
        #expect(await client.runtimeIncarnationId == Self.incarnation)
        #expect(store.isThinking)
        #expect(store.messages.isEmpty)
    }

    @Test(arguments: [
        "indicator", "text_delta", "tool_call_start", "tool_call_complete", "conversation_closed",
        "conversation_changed", "snapshot_required", "error", "body_request",
    ])
    @MainActor
    func everyKnownEventRejectsAnUndecodablePayload(type: String) async throws {
        let store = BobeStore(
            currentSelection: { Self.selection },
            conversationSnapshot: { _ in Self.snapshot("unused") }
        )
        let bundle = try #require(try DaemonClient.decodeEvent(
            StreamBundle.self,
            from: Self.event(5, type, #"{"unexpected":true}"#)
        ))

        #expect(!(await store.processBundle(bundle)))
    }

    @Test
    @MainActor
    func unknownEventTypesStayForwardCompatible() async throws {
        let store = BobeStore(currentSelection: { Self.selection })
        let bundle = try #require(try DaemonClient.decodeEvent(
            StreamBundle.self,
            from: Self.event(5, "future_event", #"{"anything":1}"#)
        ))

        #expect(await store.processBundle(bundle))
    }

    @Test
    func undecodableEnvelopeStopsTheStreamWithoutAdvancing() async throws {
        let client = DaemonClient(endpoint: .unavailable)
        let handled = EventLog()

        try await client.receiveEvents(Self.lines([
            Self.event(1, "indicator", #"{"indicator":"THINKING"}"#),
            #"data: {"event_id":2,"type":"#,
            Self.event(3, "indicator", #"{"indicator":"IDLE"}"#),
        ]), handler: { await handled.record($0) })

        #expect(await client.lastProcessedEventId == 1)
        #expect(await handled.eventIds == [1])
    }

    @Test
    func indicatorOnlyConnectionsBackOffInsteadOfReconnectingImmediately() async throws {
        let client = DaemonClient(endpoint: .unavailable)
        var delays: [Duration] = []

        for _ in 0 ..< 5 {
            try await client.receiveEvents(
                Self.lines([Self.event(nil, "indicator", #"{"indicator":"IDLE"}"#)]),
                handler: { _ in true }
            )
            delays.append(await client.reconnectDelay(connectedFor: .milliseconds(20)))
        }

        #expect(delays == [.zero, .milliseconds(500), .seconds(1), .seconds(2), .seconds(4)])
        #expect(await client.lastProcessedEventId == nil)
    }

    @Test
    func cursorProgressAndStableConnectionsResetBackoff() async throws {
        let client = DaemonClient(endpoint: .unavailable)
        for _ in 0 ..< 3 {
            _ = await client.reconnectDelay(connectedFor: nil)
        }

        try await client.receiveEvents(
            Self.lines([Self.event(4, "indicator", #"{"indicator":"IDLE"}"#)]),
            handler: { _ in true }
        )
        #expect(await client.reconnectDelay(connectedFor: .milliseconds(20)) == .zero)

        try await client.receiveEvents(
            Self.lines([Self.event(4, "indicator", #"{"indicator":"IDLE"}"#)]),
            handler: { _ in true }
        )
        #expect(await client.reconnectDelay(connectedFor: .milliseconds(20)) == .milliseconds(500))
        #expect(await client.reconnectDelay(connectedFor: DaemonClient.stableEventStreamUptime) == .zero)
    }

    private static func lines(_ values: [String]) -> AsyncStream<String> {
        AsyncStream { continuation in
            for value in values {
                continuation.yield(value)
            }
            continuation.finish()
        }
    }

    private static func event(_ id: UInt64?, _ type: String, _ payload: String) -> String {
        let cursor = id.map {
            #""event_id":\#($0),"runtime_incarnation_id":"\#(Self.incarnation.uuidString.lowercased())","#
        } ?? ""
        return #"data: {\#(cursor)"type":"\#(type)","message_id":"","payload":\#(payload)}"#
    }

    private static func snapshot(_ reply: String) -> ConversationSnapshotResponse {
        ConversationSnapshotResponse(
            turns: [
                ConversationSnapshotTurn(
                    id: "turn-b",
                    messageId: "msg_b",
                    role: "assistant",
                    content: reply,
                    isComplete: true,
                    createdAt: "2026-01-01T00:00:00Z"
                ),
            ],
            pendingUpdates: []
        )
    }
}

private actor EventLog {
    private(set) var eventIds: [UInt64] = []

    func record(_ bundle: StreamBundle) -> Bool {
        if let eventId = bundle.eventId {
            self.eventIds.append(eventId)
        }
        return true
    }
}
