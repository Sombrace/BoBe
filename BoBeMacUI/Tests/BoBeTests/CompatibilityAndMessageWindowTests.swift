@testable import BoBe
import Foundation
import Testing

@Suite("Unknown-phase compatibility and message window")
struct CompatibilityAndMessageWindowTests {
    @Test
    func unknownProviderPhaseBecomesCompatibilityFailure() throws {
        let data = Data(#"{"phase":"new_server_phase"}"#.utf8)
        let phase = try JSONDecoder().decode(ProviderLoginPhase.self, from: data)
        #expect(phase == .unsupported(phase: "new_server_phase"))
        #expect(phase.isTerminal)
    }

    @Test
    func messageWindowRetainsNewestMessages() {
        var messages = (0 ..< 5).map {
            ChatMessage(id: "message-\($0)", sender: .bobe, content: "\($0)")
        }
        BobeStore.trimMessages(&messages, capacity: 3)
        #expect(messages.map(\.id) == ["message-2", "message-3", "message-4"])
    }

    @Test
    func messageWindowProtectsPendingAndStreamingMessages() {
        var messages = [
            ChatMessage(id: "pending", sender: .user, content: "pending", isPending: true),
            ChatMessage(id: "old", sender: .bobe, content: "old"),
            ChatMessage(id: "stream", sender: .bobe, content: "stream", isStreaming: true),
            ChatMessage(id: "new", sender: .bobe, content: "new"),
        ]
        BobeStore.trimMessages(&messages, capacity: 3)
        #expect(messages.map(\.id) == ["pending", "stream", "new"])
    }
}
