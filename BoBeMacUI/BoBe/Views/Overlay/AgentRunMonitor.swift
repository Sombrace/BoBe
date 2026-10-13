import Foundation
import Observation

@MainActor
@Observable
final class AgentRunMonitor {
    private(set) var runs: [AgentRunWire] = []
    private(set) var isWorking = false
    private(set) var error: String?
    var appliesAfterCurrentTask = false

    @ObservationIgnored private var inputId = UUID()
    @ObservationIgnored private var previousGuidance: String?

    var activeRun: AgentRunWire? {
        self.runs.first { !$0.queued && $0.kind != .voice && $0.kind != .background }
    }

    var canStop: Bool {
        guard let run = self.activeRun else { return false }
        return !run.stopping && !self.isWorking
    }

    func watch(_ selection: EndpointSelection) async {
        self.runs = []
        self.error = nil
        while !Task.isCancelled {
            await self.refresh(selection)
            do { try await Task.sleep(for: .seconds(2)) } catch { return }
        }
    }

    func refresh(_ selection: EndpointSelection) async {
        do {
            let runs = try await DaemonClient(endpoint: selection.endpoint).agentRuns()
            guard EndpointStore.shared.selection == selection else { return }
            self.runs = runs
        } catch {
            guard !Task.isCancelled, EndpointStore.shared.selection == selection else { return }
            self.runs = []
        }
    }

    func stop(_ selection: EndpointSelection) async {
        guard let run = self.activeRun, !self.isWorking, EndpointStore.shared.selection == selection else { return }
        self.isWorking = true
        defer { self.isWorking = false }
        do {
            try await DaemonClient(endpoint: selection.endpoint).stopAgentRun(run.id)
            guard EndpointStore.shared.selection == selection else { return }
            self.error = nil
            await self.refresh(selection)
        } catch {
            guard EndpointStore.shared.selection == selection else { return }
            self.error = error.localizedDescription
        }
    }

    func guide(_ selection: EndpointSelection, text: String) async -> Bool {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let run = self.activeRun, !run.stopping, !self.isWorking, !trimmed.isEmpty,
              EndpointStore.shared.selection == selection
        else { return false }
        let signature = "\(run.id):\(self.appliesAfterCurrentTask):\(trimmed)"
        if self.previousGuidance != signature {
            self.inputId = UUID()
            self.previousGuidance = signature
        }
        self.isWorking = true
        defer { self.isWorking = false }
        do {
            try await DaemonClient(endpoint: selection.endpoint).guideAgentRun(
                run.id, text: trimmed, followUp: self.appliesAfterCurrentTask, inputId: self.inputId
            )
            guard EndpointStore.shared.selection == selection else { return false }
            self.error = nil
            return true
        } catch {
            guard EndpointStore.shared.selection == selection else { return false }
            self.error = error.localizedDescription
            return false
        }
    }
}
