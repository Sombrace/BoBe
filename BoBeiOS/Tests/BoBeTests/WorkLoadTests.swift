@testable import BoBe
import Foundation
import Testing

@Suite("iPhone work and memory loading")
struct WorkLoadTests {
    @Test
    func failedGoalsAreNotReportedAsEmpty() async throws {
        let harness = Harness()
        defer { harness.stop() }
        harness.link().fail("GET /goals")
        try await harness.connect()
        await harness.model.loadWork()
        #expect(harness.model.work.goalsLoad == PhoneWork.LoadState(loaded: false, failed: true))
        #expect(harness.model.work.jobsLoad == PhoneWork.LoadState(loaded: true, failed: false))
        #expect(harness.model.workFailed)
    }

    @Test
    func staleGoalsStayVisibleWithAnError() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        link.reply("GET /goals", Fixture.json(["goals": [Fixture.goal("g1")], "active_count": 1]))
        try await harness.connect()
        await harness.model.loadWork()
        #expect(harness.model.work.goals.map(\.id) == ["g1"])

        link.fail("GET /goals")
        await harness.model.loadWork()
        #expect(harness.model.work.goals.map(\.id) == ["g1"])
        #expect(harness.model.work.goalsLoad == PhoneWork.LoadState(loaded: true, failed: true))
    }

    @Test
    func failedLoadsRetryAfterReconnecting() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        link.fail("GET /goals")
        link.fail("GET /memory")
        try await harness.connect()
        await harness.model.loadWork()
        await harness.model.work.loadMemory()
        #expect(harness.model.work.memoryLoad.failed)
        #expect(harness.model.work.memory == nil)

        link.reply("GET /goals", Fixture.json(["goals": [Fixture.goal("g1")], "active_count": 1]))
        link.reply("GET /memory", Fixture.json(["content": "# Me", "bytes": 4]))
        harness.model.reconnect()
        try await eventually { harness.model.work.goalsLoad.loaded && harness.model.work.memoryLoad.loaded }
        #expect(!harness.model.workFailed)
        #expect(harness.model.work.memory == "# Me")
    }

    @Test
    func failedQuestionRefreshIsReported() async throws {
        let harness = Harness()
        defer { harness.stop() }
        harness.link().fail("GET /owner-input")
        try await harness.connect()
        try await eventually { harness.model.questionsLoad.failed }
        #expect(harness.model.workFailed)
    }
}
