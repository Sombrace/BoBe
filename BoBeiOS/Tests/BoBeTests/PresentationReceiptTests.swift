@testable import BoBe
import Foundation
import SwiftUI
import Testing
import UIKit

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
    func presentationNeedsHalfOfWhatFitsOnScreen() {
        let viewport: CGFloat = 640
        let reply = viewport * 2 + 80

        #expect(MessagePresentation.isPresented(visibleHeight: viewport, rowHeight: reply, viewportHeight: viewport))
        #expect(MessagePresentation.isPresented(visibleHeight: viewport / 2, rowHeight: reply, viewportHeight: viewport))
        #expect(!MessagePresentation.isPresented(visibleHeight: 24, rowHeight: reply, viewportHeight: viewport))
        #expect(MessagePresentation.isPresented(visibleHeight: 60, rowHeight: 100, viewportHeight: viewport))
        #expect(!MessagePresentation.isPresented(visibleHeight: 40, rowHeight: 100, viewportHeight: viewport))
        #expect(!MessagePresentation.isPresented(visibleHeight: 0, rowHeight: 0, viewportHeight: viewport))
    }

    @Test
    func replyTallerThanTwiceTheScreenIsPresented() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        let turn = UUID()
        let messageId = "msg_" + turn.uuidString.lowercased().replacingOccurrences(of: "-", with: "")
        let receipt = "POST /conversation/\(turn.uuidString)/presented"
        let reply = (1 ... 120).map { "Step \($0): keep the plan moving." }.joined(separator: "\n")
        link.reply("GET /conversation/current", Fixture.snapshot([Fixture.turn(messageId, role: "assistant", content: reply)]))
        try await harness.connect()
        let message = try #require(harness.model.thread.first)

        let screen = CGSize(width: 390, height: 420)
        let row = UIHostingController(rootView: MessageRow(model: harness.model, message: message).phoneThemed())
        #expect(row.sizeThatFits(in: CGSize(width: screen.width, height: .greatestFiniteMagnitude)).height > screen.height * 2)

        let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let window = UIWindow(windowScene: scene)
        window.frame = CGRect(origin: .zero, size: screen)
        window.rootViewController = UIHostingController(
            rootView: ThreadView(model: harness.model).environment(\.scenePhase, .active).phoneThemed()
        )
        window.isHidden = false
        defer { window.isHidden = true }
        try await eventually(.seconds(10)) { link.requests(receipt).count == 1 }
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
