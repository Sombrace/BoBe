import Foundation

struct ThreadMessage: Identifiable, Equatable, Sendable {
    enum Author: Sendable {
        case me
        case bobe
    }

    enum Delivery: Sendable {
        case sending
        case sent
        case failed
    }

    var id: String
    let author: Author
    var text: String
    var isComplete: Bool
    var delivery: Delivery
    var reachedOut = false
    let date: Date

    init(
        id: String,
        author: Author,
        text: String,
        isComplete: Bool,
        delivery: Delivery,
        date: Date
    ) {
        self.id = id
        self.author = author
        self.text = text
        self.isComplete = isComplete
        self.delivery = delivery
        self.date = date
    }

    init?(turn: ConversationSnapshotTurn) {
        switch turn.role {
        case "user": self.author = .me
        case "assistant": self.author = .bobe
        default: return nil
        }
        self.id = turn.messageId
        self.text = turn.content
        self.isComplete = turn.isComplete
        self.delivery = .sent
        self.date = Self.parse(turn.createdAt) ?? .now
    }

    var presentationId: UUID? {
        guard self.author == .bobe else { return nil }
        let prefix = self.id.hasPrefix("msg_") ? "msg_" : self.id.hasPrefix("voice_") ? "voice_" : nil
        guard let prefix else { return nil }
        let raw = String(self.id.dropFirst(prefix.count))
        if let id = UUID(uuidString: raw) { return id }
        guard raw.count == 32, raw.allSatisfy(\.isHexDigit) else { return nil }
        return UUID(uuidString: [
            String(raw.prefix(8)), String(raw.dropFirst(8).prefix(4)),
            String(raw.dropFirst(12).prefix(4)), String(raw.dropFirst(16).prefix(4)),
            String(raw.suffix(12)),
        ].joined(separator: "-"))
    }

    static func ordered(_ messages: [ThreadMessage]) -> [ThreadMessage] {
        var previous: Author?
        return messages.sorted { $0.date < $1.date }.map { message in
            var marked = message
            marked.reachedOut = message.author == .bobe && previous != .me
            previous = message.author
            return marked
        }
    }

    private static func parse(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: value) { return date }
        return ISO8601DateFormatter().date(from: value)
    }
}
