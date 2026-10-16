@testable import BoBe
import Foundation
import Testing

@MainActor
@Suite("iPhone presentation receipts")
struct PresentationReceiptTests {
    @Test
    func failedReceiptIsRetriedAfterReconnecting() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        let turn = UUID()
        let messageId = "msg_" + turn.uuidString.lowercased().replacingOccurrences(of: "-", with: "")
        let receipt = "POST /conversation/\(turn.uuidString)/presented"
        link.reply("GET /conversation/current", Fixture.snapshot([Fixture.turn(messageId, role: "assistant", content: "Hi")]))
        link.fail(receipt)
        try await harness.connect()

        let message = try #require(harness.model.thread.first)
        harness.model.presented(message)
        try await eventually { link.requests(receipt).count == 1 }
        await pause()

        link.reply(receipt, Data())
        harness.model.reconnect()
        try await eventually { link.requests(receipt).count == 2 }

        harness.model.presented(message)
        await pause()
        #expect(link.requests(receipt).count == 2)
    }

    @Test
    func answerWaitsForTheQuestionReceipt() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        let id = UUID()
        let receipt = "POST /owner-input/\(id.uuidString)/presented"
        let answer = "POST /owner-input/\(id.uuidString)/answer"
        link.reply("GET /owner-input", Fixture.json([Fixture.question(id)]))
        link.reply(answer, Fixture.json(Fixture.question(id, status: "answered")))
        link.fail(receipt)
        try await harness.connect()
        try await eventually { harness.model.questions.count == 1 }

        let question = try #require(harness.model.questions.first)
        harness.model.presented(question)
        try await eventually { link.requests(receipt).count == 1 }
        await pause()
        #expect(await harness.model.answer(question, with: "yes") == false)
        #expect(link.requests(answer).isEmpty)

        link.reply(receipt, Data())
        #expect(await harness.model.answer(question, with: "yes"))
        #expect(link.requests(answer).count == 1)
        #expect(harness.model.questions.isEmpty)
    }

    @Test
    func retriedAnswerKeepsItsResponseIdentity() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        let id = UUID()
        let answer = "POST /owner-input/\(id.uuidString)/answer"
        link.reply("GET /owner-input", Fixture.json([Fixture.question(id)]))
        link.fail(answer)
        try await harness.connect()
        try await eventually { harness.model.questions.count == 1 }
        let question = try #require(harness.model.questions.first)

        #expect(await harness.model.answer(question, with: "yes") == false)
        #expect(await harness.model.answer(question, with: "no") == false)
        link.reply(answer, Fixture.json(Fixture.question(id, status: "answered")))
        #expect(await harness.model.answer(question, with: "no"))

        let identities = link.requests(answer).compactMap { $0.json["response_id"] as? String }
        #expect(identities.count == 3)
        #expect(identities[0] != identities[1])
        #expect(identities[1] == identities[2])
    }
}
