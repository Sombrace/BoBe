import Foundation
import Security

struct BrainRecord: Codable, Sendable, Equatable {
    var origins: [URL]
    let pin: Data
}

struct BrainPairing: Codable, Sendable, Equatable {
    var record: BrainRecord
    let credential: String
}

enum BrainVaultError: Error {
    case unavailable(OSStatus)
}

enum BrainVault {
    private static let service = "com.bobe.ios.brain"
    private static let account = "paired"

    static func load() -> BrainPairing? {
        var query = self.query
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
              let data = result as? Data
        else { return nil }
        return try? JSONDecoder().decode(BrainPairing.self, from: data)
    }

    static func save(_ pairing: BrainPairing) throws {
        let data = try JSONEncoder().encode(pairing)
        SecItemDelete(self.query as CFDictionary)
        var item = self.query
        item[kSecValueData as String] = data
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(item as CFDictionary, nil)
        guard status == errSecSuccess else { throw BrainVaultError.unavailable(status) }
    }

    static func remove() {
        SecItemDelete(self.query as CFDictionary)
    }

    private static var query: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: self.service,
            kSecAttrAccount as String: self.account,
        ]
    }
}
