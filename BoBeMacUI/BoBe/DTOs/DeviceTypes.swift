import Foundation

struct DeviceWire: Decodable, Identifiable, Sendable, Equatable {
    let id: String
    let kind: String
    let label: String
    let revokedAt: String?

    enum CodingKeys: String, CodingKey {
        case id, kind, label
        case revokedAt = "revoked_at"
    }
}

struct PhoneInvitationWire: Decodable, Sendable, Equatable {
    let code: String
    let expiresInSeconds: Int
    let pin: String
    let origins: [String]
    let link: String

    enum CodingKeys: String, CodingKey {
        case code, pin, origins, link
        case expiresInSeconds = "expires_in_seconds"
    }
}
