import Foundation

enum ReachoutKind: Equatable, Sendable {
    case answer
    case thought
    case report

    init(origin: AssistantResponseOrigin?, run: AgentRunKindWire?) {
        if origin == .userInitiated {
            self = .answer
            return
        }
        switch run {
        case .delivery, .background: self = .report
        case .user, .voice: self = .answer
        case .proactive, nil: self = .thought
        }
    }

    var symbol: String? {
        switch self {
        case .answer: nil
        case .thought: "sparkle"
        case .report: "checkmark.seal.fill"
        }
    }

    var label: String? {
        switch self {
        case .answer: nil
        case .thought: L10n.tr("overlay.reachout.kind.thought")
        case .report: L10n.tr("overlay.reachout.kind.report")
        }
    }

    var waitsWhenMissed: Bool {
        self != .answer
    }

    static func carrier(ofMessage id: String, in runs: [AgentRunWire]) -> AgentRunKindWire? {
        if let runID = self.runID(forMessage: id), let run = runs.first(where: { $0.id == runID }) {
            return run.kind
        }
        return runs.first(where: { !$0.queued && $0.kind == .delivery })?.kind
    }

    static func runID(forMessage id: String) -> UUID? {
        let prefix = "msg_"
        guard id.hasPrefix(prefix) else { return UUID(uuidString: id) }
        let hex = id.dropFirst(prefix.count)
        guard hex.count == 32, hex.allSatisfy(\.isHexDigit) else { return nil }
        var dashed = ""
        for (offset, character) in hex.enumerated() {
            if [8, 12, 16, 20].contains(offset) { dashed.append("-") }
            dashed.append(character)
        }
        return UUID(uuidString: dashed)
    }
}

struct ReachoutInbox: Equatable, Sendable {
    private(set) var waiting: [String] = []

    var count: Int { self.waiting.count }
    var isEmpty: Bool { self.waiting.isEmpty }
    var latest: String? { self.waiting.last }

    mutating func hold(_ id: String) {
        guard !self.waiting.contains(id) else { return }
        self.waiting.append(id)
    }

    mutating func settle(_ id: String) {
        self.waiting.removeAll { $0 == id }
    }

    mutating func settleAll() {
        self.waiting.removeAll()
    }

    mutating func prune(keeping ids: Set<String>) {
        self.waiting.removeAll { !ids.contains($0) }
    }
}
