import Foundation

struct ReplyPlayback: Equatable, Sendable {
    enum Outcome: Equatable, Sendable {
        case heard
        case unheard
    }

    let turnId: String
    private(set) var renderedSegments = 0
    private(set) var failed = false

    init(turnId: String) {
        self.turnId = turnId
    }

    mutating func noteRendered() {
        self.renderedSegments += 1
    }

    mutating func noteFailure() {
        self.failed = true
    }

    func outcome(drained: Bool) -> Outcome {
        drained && !self.failed && self.renderedSegments > 0 ? .heard : .unheard
    }
}
