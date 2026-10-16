import Foundation
import Security

struct BrainRecord: Codable, Sendable, Equatable {
    var origins: [URL]
    let pin: Data
    var endpointId: UUID?
    var authorityId: UUID?
    var authorityEpoch: Int?
}

struct BrainPairing: Codable, Sendable, Equatable {
    var record: BrainRecord
    let credential: String
}

enum BrainVaultError: Error {
    case unavailable(OSStatus)
}

protocol SecretStore: Sendable {
    func read(_ account: String) -> Data?
    func add(_ data: Data, account: String) -> OSStatus
    func update(_ data: Data, account: String) -> OSStatus
    func delete(_ account: String)
}

struct BrainVault: Sendable {
    static let keychain = BrainVault(store: KeychainSecrets(service: "com.bobe.ios.brain"))

    private static let pairedAccount = "paired"
    private static let retiringAccount = "retiring"

    let store: any SecretStore

    func load() -> BrainPairing? {
        self.store.read(Self.pairedAccount).flatMap { try? JSONDecoder().decode(BrainPairing.self, from: $0) }
    }

    func save(_ pairing: BrainPairing) throws {
        try self.write(JSONEncoder().encode(pairing), account: Self.pairedAccount)
    }

    func remove() {
        self.store.delete(Self.pairedAccount)
    }

    func retiring() -> [BrainPairing] {
        self.store.read(Self.retiringAccount).flatMap { try? JSONDecoder().decode([BrainPairing].self, from: $0) } ?? []
    }

    func saveRetiring(_ pairings: [BrainPairing]) throws {
        guard !pairings.isEmpty else {
            self.store.delete(Self.retiringAccount)
            return
        }
        try self.write(JSONEncoder().encode(pairings), account: Self.retiringAccount)
    }

    private func write(_ data: Data, account: String) throws {
        var status = self.store.update(data, account: account)
        if status == errSecItemNotFound {
            status = self.store.add(data, account: account)
        }
        guard status == errSecSuccess else { throw BrainVaultError.unavailable(status) }
    }
}

struct KeychainSecrets: SecretStore {
    let service: String

    func read(_ account: String) -> Data? {
        var query = self.query(account)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess else { return nil }
        return result as? Data
    }

    func add(_ data: Data, account: String) -> OSStatus {
        var item = self.query(account)
        item[kSecValueData as String] = data
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        return SecItemAdd(item as CFDictionary, nil)
    }

    func update(_ data: Data, account: String) -> OSStatus {
        SecItemUpdate(self.query(account) as CFDictionary, [kSecValueData as String: data] as CFDictionary)
    }

    func delete(_ account: String) {
        SecItemDelete(self.query(account) as CFDictionary)
    }

    private func query(_ account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: self.service,
            kSecAttrAccount as String: account,
        ]
    }
}
