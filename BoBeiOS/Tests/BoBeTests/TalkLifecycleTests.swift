@testable import BoBe
import Foundation
import SwiftUI
import Testing

@Suite("iPhone hold-to-talk lifecycle")
struct TalkLifecycleTests {
    @Test
    func backgroundingStopsListening() {
        let talk = HoldToTalk(state: .listening)
        let harness = Harness(talk: talk)
        harness.model.scenePhaseChanged(.background)
        #expect(talk.state == .idle)
    }

    @Test
    func unpairingStopsListening() {
        let talk = HoldToTalk(state: .listening)
        let harness = Harness(talk: talk)
        defer { harness.stop() }
        harness.model.unpair()
        #expect(talk.state == .idle)
    }

    @Test
    func losingThePairingStopsListening() async throws {
        let talk = HoldToTalk(state: .listening)
        let harness = Harness(talk: talk)
        defer { harness.stop() }
        harness.link().fail("POST /v1/hello", .unauthorized)
        harness.model.start()
        try await eventually { harness.model.link == .removed }
        #expect(talk.state == .idle)
    }
}
