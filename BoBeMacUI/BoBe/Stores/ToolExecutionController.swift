import Foundation

@MainActor
final class ToolExecutionController {
    typealias StateMutation = (inout BobeContext) -> Void
    typealias StateApplier = (StateMutation) -> Void

    private let applyStateMutation: StateApplier
    private var cleanupTasks: [String: Task<Void, Never>] = [:]

    init(applyStateMutation: @escaping StateApplier) {
        self.applyStateMutation = applyStateMutation
    }

    deinit {
        self.cleanupTasks.values.forEach { $0.cancel() }
    }

    func process(_ payload: AnyCodablePayload) {
        if let start = try? payload.decode(as: ToolCallStartPayload.self),
           start.status == ToolCallStatusWire.start {
            self.handleStart(start)
            return
        }

        if let complete = try? payload.decode(as: ToolCallCompletePayload.self),
           complete.status == ToolCallStatusWire.complete {
            self.handleComplete(complete)
        }
    }

    private func handleStart(_ start: ToolCallStartPayload) {
        self.cleanupTasks[start.toolCallId]?.cancel()
        self.cleanupTasks[start.toolCallId] = nil

        let execution = ToolExecution(
            toolName: start.toolName,
            toolCallId: start.toolCallId,
            status: .running
        )
        self.applyStateMutation { $0.toolExecutions.append(execution) }
    }

    private func handleComplete(_ complete: ToolCallCompletePayload) {
        self.applyStateMutation { ctx in
            ctx.toolExecutions = ctx.toolExecutions.map { t in
                guard t.toolCallId == complete.toolCallId else { return t }
                var updated = t
                updated.status = complete.success ? .success : .error
                return updated
            }
        }

        let completedId = complete.toolCallId
        self.cleanupTasks[completedId]?.cancel()
        let cleanupTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(StoreTiming.toolCompletionLingerSeconds))
            guard let self else { return }
            self.applyStateMutation { ctx in
                ctx.toolExecutions.removeAll { $0.toolCallId == completedId && $0.status != .running }
            }
            self.cleanupTasks[completedId] = nil
        }
        self.cleanupTasks[completedId] = cleanupTask
    }
}
