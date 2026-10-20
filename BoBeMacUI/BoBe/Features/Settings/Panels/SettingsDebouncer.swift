import Foundation

@Observable
final class SettingsDebouncer {
    static let debounceSeconds: Double = 0.6
    static let toastSeconds: Double = 2.4

    private var saveTask: Task<Void, Never>?
    private var inFlightTask: Task<Void, Never>?
    private var pendingWork: (@MainActor () async -> Void)?
    private var toastTask: Task<Void, Never>?
    private var generation: UInt64 = 0
    private var persistGeneration: UInt64 = 0

    var hasPendingSave: Bool {
        self.pendingWork != nil || self.inFlightTask != nil
    }

    func cancelPendingSave() {
        self.generation &+= 1
        self.saveTask?.cancel()
        self.saveTask = nil
        self.pendingWork = nil
        self.cancelToast()
    }

    func debounce(_ work: @escaping @MainActor () async -> Void) {
        self.saveTask?.cancel()
        self.generation &+= 1
        let generation = self.generation
        self.pendingWork = work
        self.saveTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(Self.debounceSeconds))
            guard !Task.isCancelled, let self else { return }
            await self.drainInFlight()
            guard !Task.isCancelled, generation == self.generation, let work = self.pendingWork else { return }
            self.pendingWork = nil
            self.persistGeneration &+= 1
            let persistGeneration = self.persistGeneration
            let persist = Task { await work() }
            self.inFlightTask = persist
            await persist.value
            if persistGeneration == self.persistGeneration { self.inFlightTask = nil }
            if generation == self.generation { self.saveTask = nil }
        }
    }

    func scheduleToastClear(_ clear: @escaping @MainActor () -> Void) {
        self.toastTask?.cancel()
        self.toastTask = Task {
            try? await Task.sleep(for: .seconds(Self.toastSeconds))
            guard !Task.isCancelled else { return }
            clear()
        }
    }

    func cancelToast() {
        self.toastTask?.cancel()
        self.toastTask = nil
    }

    private func drainInFlight() async {
        while let task = self.inFlightTask {
            let generation = self.persistGeneration
            await task.value
            if generation == self.persistGeneration { self.inFlightTask = nil }
        }
    }

    func flush() async {
        self.generation &+= 1
        self.saveTask?.cancel()
        self.saveTask = nil
        await self.drainInFlight()
        guard let work = self.pendingWork else { return }
        self.pendingWork = nil
        self.persistGeneration &+= 1
        let persistGeneration = self.persistGeneration
        let persist = Task { await work() }
        self.inFlightTask = persist
        await persist.value
        if persistGeneration == self.persistGeneration { self.inFlightTask = nil }
    }

    func runPersist(
        setError: (String?) -> Void,
        setSaved: @escaping (String?) -> Void,
        onSuccess: (@MainActor (SettingsUpdateResponse) async -> Void)? = nil,
        update: @MainActor (SettingsUpdateRequest) async throws -> SettingsUpdateResponse,
        isCurrent: @escaping () -> Bool,
        build: (inout SettingsUpdateRequest) -> Void
    ) async {
        var req = SettingsUpdateRequest()
        build(&req)
        do {
            guard isCurrent() else { return }
            let resp = try await update(req)
            guard isCurrent() else { return }
            if resp.persistFailed == true {
                setError(L10n.tr("settings.shared.action.persist_failed"))
                setSaved(nil)
            } else {
                setError(nil)
                setSaved(resp.message)
                self.scheduleToastClear { if isCurrent() { setSaved(nil) } }
                if let onSuccess { await onSuccess(resp) }
            }
        } catch {
            guard isCurrent() else { return }
            setError(error.localizedDescription)
            setSaved(nil)
        }
    }
}
