import Foundation

enum ReceiptOutcome: Sendable, Equatable {
    case confirmed
    case retry
    case dropped
}

final class ReceiptBook<Key: Hashable & Sendable> {
    private let deliver: @MainActor (Key) async -> ReceiptOutcome
    private var confirmed: Set<Key> = []
    private var waiting: Set<Key> = []
    private var inFlight: [Key: Task<ReceiptOutcome, Never>] = [:]
    private var generation = 0

    init(deliver: @escaping @MainActor (Key) async -> ReceiptOutcome) {
        self.deliver = deliver
    }

    func isConfirmed(_ key: Key) -> Bool {
        self.confirmed.contains(key)
    }

    @discardableResult
    func present(_ key: Key) async -> ReceiptOutcome {
        if self.confirmed.contains(key) { return .confirmed }
        if let attempt = self.inFlight[key] { return await attempt.value }
        let generation = self.generation
        self.waiting.insert(key)
        let deliver = self.deliver
        let attempt = Task { await deliver(key) }
        self.inFlight[key] = attempt
        let outcome = await attempt.value
        guard generation == self.generation else { return outcome }
        self.inFlight[key] = nil
        switch outcome {
        case .confirmed:
            self.confirmed.insert(key)
            self.waiting.remove(key)
        case .dropped:
            self.waiting.remove(key)
        case .retry:
            break
        }
        return outcome
    }

    func flush() {
        for key in self.waiting where self.inFlight[key] == nil {
            Task { await self.present(key) }
        }
    }

    func discard(where shouldDiscard: (Key) -> Bool) {
        self.waiting = self.waiting.filter { !shouldDiscard($0) }
    }

    func reset() {
        self.generation += 1
        self.confirmed = []
        self.waiting = []
        self.inFlight = [:]
    }
}
