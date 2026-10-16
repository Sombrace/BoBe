import Foundation

struct ConversationSnapshotResponse: Decodable, Sendable {
    let turns: [ConversationSnapshotTurn]
    let pendingUpdates: [ConversationSnapshotTurn]

    enum CodingKeys: String, CodingKey {
        case turns
        case pendingUpdates = "pending_updates"
    }
}

struct ConversationSnapshotTurn: Decodable, Sendable {
    let id: String
    let messageId: String
    let role: String
    let content: String
    let isComplete: Bool
    let createdAt: String

    enum CodingKeys: String, CodingKey {
        case id
        case messageId = "message_id"
        case role
        case content
        case isComplete = "is_complete"
        case createdAt = "created_at"
    }
}
