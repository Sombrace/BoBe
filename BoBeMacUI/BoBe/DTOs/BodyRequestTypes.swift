import Foundation

enum BodyRequestWire {
    static let actionsHeader = "BoBe-Body-Actions"
    static let screenRead = "screen.read"
    static let offeredActions = screenRead
}

struct BodyRequestPayload: Codable, Sendable, Equatable {
    let requestId: String
    let action: String
    let deadlineMs: UInt64

    enum CodingKeys: String, CodingKey {
        case requestId = "request_id"
        case action
        case deadlineMs = "deadline_ms"
    }
}

struct ScreenReadingWire: Codable, Sendable, Equatable {
    let text: String
    let application: String?
    let windowTitle: String?

    enum CodingKeys: String, CodingKey {
        case text
        case application
        case windowTitle = "window_title"
    }
}

struct BodyReply: Encodable, Sendable, Equatable {
    enum Status: String, Encodable, Sendable {
        case completed
        case declined
        case failed
    }

    let status: Status
    let result: ScreenReadingWire?
    let reason: String?

    static func completed(_ reading: ScreenReadingWire) -> Self {
        Self(status: .completed, result: reading, reason: nil)
    }

    static func declined(_ reason: String) -> Self {
        Self(status: .declined, result: nil, reason: reason)
    }

    static func failed(_ reason: String) -> Self {
        Self(status: .failed, result: nil, reason: reason)
    }
}
