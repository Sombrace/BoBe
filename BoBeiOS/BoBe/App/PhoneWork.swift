import Foundation
import Observation
import SwiftUI

@MainActor
@Observable
final class PhoneWork {
    struct LoadState: Equatable {
        var loaded = false
        var failed = false

        mutating func record(_ succeeded: Bool) {
            if succeeded {
                self.loaded = true
            }
            self.failed = !succeeded
        }
    }

    private(set) var goals: [Goal] = []
    private(set) var jobs: [AgentJobWire] = []
    private(set) var schedules: [AgentScheduleWire] = []
    private(set) var memory: String?
    private(set) var goalsLoad = LoadState()
    private(set) var jobsLoad = LoadState()
    private(set) var schedulesLoad = LoadState()
    private(set) var memoryLoad = LoadState()

    @ObservationIgnored private let client: @MainActor () -> (any BrainLink)?
    @ObservationIgnored private var generation = 0

    init(client: @escaping @MainActor () -> (any BrainLink)?) {
        self.client = client
    }

    var failed: Bool {
        self.goalsLoad.failed || self.jobsLoad.failed || self.schedulesLoad.failed
    }

    func load() async {
        guard let client = self.client() else { return }
        let generation = self.generation
        async let goals: GoalListResponse? = try? client.get(["goals"])
        async let jobs: [AgentJobWire]? = try? client.get(["agent", "jobs"])
        async let schedules: [AgentScheduleWire]? = try? client.get(["agent", "schedules"])
        let (loadedGoals, loadedJobs, loadedSchedules) = await (goals, jobs, schedules)
        guard generation == self.generation else { return }
        if let loadedGoals { self.goals = loadedGoals.goals }
        if let loadedJobs { self.jobs = loadedJobs }
        if let loadedSchedules { self.schedules = loadedSchedules }
        self.goalsLoad.record(loadedGoals != nil)
        self.jobsLoad.record(loadedJobs != nil)
        self.schedulesLoad.record(loadedSchedules != nil)
    }

    func loadMemory() async {
        guard let client = self.client() else { return }
        let generation = self.generation
        let response: MemoryResponse? = try? await client.get(["memory"])
        guard generation == self.generation else { return }
        if let response { self.memory = response.content }
        self.memoryLoad.record(response != nil)
    }

    func retryFailed() {
        if self.failed {
            Task { await self.load() }
        }
        if self.memoryLoad.failed {
            Task { await self.loadMemory() }
        }
    }

    func setGoal(_ goal: Goal, paused: Bool) async {
        guard let client = self.client() else { return }
        if let updated: Goal = try? await client.send(
            ["goals", goal.id],
            method: "PATCH",
            body: GoalUpdateRequest(status: paused ? .paused : .active)
        ), let index = self.goals.firstIndex(where: { $0.id == goal.id }) {
            withAnimation(BobeMotion.standard) { self.goals[index] = updated }
        }
    }

    @discardableResult
    func completeGoal(_ goal: Goal) async -> Bool {
        guard let client = self.client() else { return false }
        do {
            try await client.call(["goals", goal.id, "complete"])
            return true
        } catch {
            return false
        }
    }

    func createGoal(title: String, why: String) async -> Bool {
        guard let client = self.client() else { return false }
        let reason = why.trimmingCharacters(in: .whitespacesAndNewlines)
        do {
            let goal: Goal = try await client.send(
                ["goals"],
                body: GoalCreateRequest(
                    title: title.trimmingCharacters(in: .whitespacesAndNewlines),
                    whyItMatters: reason.isEmpty ? nil : reason
                )
            )
            withAnimation(BobeMotion.standard) { self.goals.insert(goal, at: 0) }
            return true
        } catch {
            return false
        }
    }

    func reset() {
        self.generation += 1
        self.goals = []
        self.jobs = []
        self.schedules = []
        self.memory = nil
        self.goalsLoad = LoadState()
        self.jobsLoad = LoadState()
        self.schedulesLoad = LoadState()
        self.memoryLoad = LoadState()
    }
}
