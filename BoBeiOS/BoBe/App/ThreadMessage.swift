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
        case unconfirmed
    }

    var id: String
    let author: Author
    var text: String
    var isComplete: Bool
    var isStreaming = false
    var delivery: Delivery
    var reachedOut = false
    var requestId: UUID?
    var replyId: String?
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

    static func outgoing(_ text: String, requestId: UUID = UUID(), date: Date = .now) -> ThreadMessage {
        var message = ThreadMessage(
            id: "local-" + requestId.uuidString,
            author: .me,
            text: text,
            isComplete: true,
            delivery: .sending,
            date: date
        )
        message.requestId = requestId
        return message
    }

    var isLocal: Bool {
        self.requestId != nil
    }

    var presentationId: UUID? {
        guard self.author == .bobe else { return nil }
        return Self.presentationId(for: self.id)
    }

    static func presentationId(for id: String) -> UUID? {
        let prefix = id.hasPrefix("msg_") ? "msg_" : id.hasPrefix("voice_") ? "voice_" : nil
        guard let prefix else { return nil }
        let raw = String(id.dropFirst(prefix.count))
        if let turn = UUID(uuidString: raw) { return turn }
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

    static func merge(canonical: [ThreadMessage], local: [ThreadMessage]) -> [ThreadMessage] {
        let known = Set(canonical.map(\.id))
        var spoken: [String: Int] = [:]
        for message in canonical where message.author == .me {
            spoken[message.text, default: 0] += 1
        }
        let pending = local.filter { message in
            guard message.delivery == .sent else { return true }
            let available = spoken[message.text, default: 0]
            if available > 0 {
                spoken[message.text] = available - 1
            }
            if let reply = message.replyId, known.contains(reply) { return false }
            return available == 0
        }
        return self.ordered(canonical) + pending
    }

    private static func parse(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = fractional.date(from: value) { return date }
        return ISO8601DateFormatter().date(from: value)
    }
}
