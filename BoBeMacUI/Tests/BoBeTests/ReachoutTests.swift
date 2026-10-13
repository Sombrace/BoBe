@testable import BoBe
import Foundation
import Testing

@Suite("Reachout")
struct ReachoutTests {
    @Test
    func repliesToTheOwnerAreAnswersWhateverRunCarriedThem() {
        #expect(ReachoutKind(origin: .userInitiated, run: .proactive) == .answer)
        #expect(ReachoutKind(origin: .userInitiated, run: nil) == .answer)
        #expect(ReachoutKind(origin: .proactive, run: .voice) == .answer)
    }

    @Test
    func initiativeSplitsIntoThoughtsAndReports() {
        #expect(ReachoutKind(origin: .proactive, run: .proactive) == .thought)
        #expect(ReachoutKind(origin: .proactive, run: nil) == .thought)
        #expect(ReachoutKind(origin: .proactive, run: .delivery) == .report)
        #expect(ReachoutKind(origin: nil, run: .background) == .report)
    }

    @Test
    func onlyInitiativeWaitsWhenMissed() {
        #expect(!ReachoutKind.answer.waitsWhenMissed)
        #expect(ReachoutKind.thought.waitsWhenMissed)
        #expect(ReachoutKind.report.waitsWhenMissed)
    }

    @Test
    func messageIdentifiersMapToTheirRun() throws {
        let run = try #require(UUID(uuidString: "0A1B2C3D-4E5F-4071-8293-A4B5C6D7E8F9"))
        #expect(ReachoutKind.runID(forMessage: "msg_0a1b2c3d4e5f40718293a4b5c6d7e8f9") == run)
        #expect(ReachoutKind.runID(forMessage: run.uuidString) == run)
        #expect(ReachoutKind.runID(forMessage: "msg_short") == nil)
        #expect(ReachoutKind.runID(forMessage: "msg_zz1b2c3d4e5f40718293a4b5c6d7e8f9") == nil)
    }

    @Test
    func carrierPrefersTheRunNamedByTheMessageThenAnActiveDelivery() throws {
        let turn = try #require(UUID(uuidString: "0A1B2C3D-4E5F-4071-8293-A4B5C6D7E8F9"))
        let message = "msg_0a1b2c3d4e5f40718293a4b5c6d7e8f9"
        let proactive = AgentRunWire(id: turn, kind: .proactive, stopping: false, queued: false)
        let delivery = AgentRunWire(id: UUID(), kind: .delivery, stopping: false, queued: false)
        let queuedDelivery = AgentRunWire(id: UUID(), kind: .delivery, stopping: false, queued: true)
        let user = AgentRunWire(id: UUID(), kind: .user, stopping: false, queued: false)
        #expect(ReachoutKind.carrier(ofMessage: message, in: [delivery, proactive]) == .proactive)
        #expect(ReachoutKind.carrier(ofMessage: message, in: [user, delivery]) == .delivery)
        #expect(ReachoutKind.carrier(ofMessage: message, in: [user, queuedDelivery]) == nil)
        #expect(ReachoutKind.carrier(ofMessage: message, in: []) == nil)
    }

    @Test
    func inboxHoldsEachReachoutOnceInArrivalOrder() {
        var inbox = ReachoutInbox()
        inbox.hold("a")
        inbox.hold("b")
        inbox.hold("a")
        #expect(inbox.waiting == ["a", "b"])
        #expect(inbox.latest == "b")
        inbox.settle("b")
        #expect(inbox.latest == "a")
        #expect(inbox.count == 1)
        inbox.settleAll()
        #expect(inbox.isEmpty)
    }

    @Test
    func inboxForgetsReachoutsThatLeftTheConversation() {
        var inbox = ReachoutInbox()
        inbox.hold("a")
        inbox.hold("b")
        inbox.prune(keeping: ["b", "c"])
        #expect(inbox.waiting == ["b"])
    }
}
