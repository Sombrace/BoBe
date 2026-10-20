import Foundation
import Observation

@Observable
final class PairingRetirement {
    private(set) var pending: Bool

    @ObservationIgnored private let vault: BrainVault
    @ObservationIgnored private let makeLink: @MainActor (URL, BrainPairing) -> any BrainLink
    @ObservationIgnored private let active: @MainActor () -> String?
    @ObservationIgnored private var task: Task<Void, Never>?

    init(
        vault: BrainVault,
        link: @escaping @MainActor (URL, BrainPairing) -> any BrainLink,
        active: @escaping @MainActor () -> String?
    ) {
        self.vault = vault
        self.makeLink = link
        self.active = active
        self.pending = !vault.retiring().isEmpty
    }

    func save(_ pairing: BrainPairing, replacing previous: BrainPairing?) throws {
        guard let previous else {
            try self.vault.save(pairing)
            return
        }
        let queue = self.vault.retiring()
        try self.vault.saveRetiring(Self.unique(queue + [previous]))
        do {
            try self.vault.save(pairing)
        } catch {
            try? self.vault.saveRetiring(queue)
            throw error
        }
    }

    func retire(_ pairing: BrainPairing) {
        try? self.vault.saveRetiring(Self.unique(self.vault.retiring() + [pairing]))
        self.resume(including: [pairing])
    }

    func resume(including extra: [BrainPairing] = []) {
        guard !extra.isEmpty || !self.vault.retiring().isEmpty else {
            self.pending = false
            return
        }
        let previous = self.task
        self.task = Task(name: "Retire pairing") { [weak self] in
            await previous?.value
            await self?.run(including: extra)
        }
    }

    private func run(including extra: [BrainPairing]) async {
        var forgotten: Set<String> = []
        for old in Self.unique(self.vault.retiring() + extra) where old.credential != self.active() {
            if await self.forget(old) {
                forgotten.insert(old.credential)
            }
        }
        let active = self.active()
        let remaining = Self.unique(self.vault.retiring() + extra).filter {
            !forgotten.contains($0.credential) && $0.credential != active
        }
        try? self.vault.saveRetiring(remaining)
        self.pending = !remaining.isEmpty
    }

    private func forget(_ old: BrainPairing) async -> Bool {
        for origin in old.record.origins {
            let link = self.makeLink(origin, old)
            defer { link.invalidate() }
            do {
                try await link.call(["endpoints", "self"], method: "DELETE")
                return true
            } catch BrainError.unauthorized {
                return true
            } catch let BrainError.http(status, _) where (400 ..< 500).contains(status) && ![408, 429].contains(status) {
                return true
            } catch {
                continue
            }
        }
        return false
    }

    private static func unique(_ pairings: [BrainPairing]) -> [BrainPairing] {
        var seen: Set<String> = []
        return pairings.filter { seen.insert($0.credential).inserted }
    }
}
