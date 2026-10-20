import Foundation

final class AudioLifecycle {
    private var tail: Task<Void, Never>?
    private(set) var generation: UInt64 = 0

    func invalidate() {
        self.generation &+= 1
    }

    func enqueue(_ operation: @escaping @MainActor @Sendable () async -> Void) {
        let previous = self.tail
        self.tail = Task {
            await previous?.value
            await operation()
        }
    }

    func settled() async {
        while let tail = self.tail {
            await tail.value
            if self.tail == tail { return }
        }
    }
}
