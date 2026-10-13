import Foundation
import LocalAuthentication
import OSLog
import Security

private let endpointLogger = Logger(subsystem: "com.bobe.app", category: "EndpointStore")

extension Notification.Name {
    static let bobeEndpointIdentityChanged = Notification.Name("bobe.endpointIdentityChanged")
}

struct EndpointProfile: Codable, Sendable, Equatable {
    enum TrustMode: String, Codable, Sendable {
        case loopback
        case pinnedSPKI = "pinned_spki"
    }

    enum Mode: String, Codable, Sendable {
        case managedLocal = "managed_local"
        case attachedLocal = "attached_local"
        case remote
    }

    let id: UUID
    let mode: Mode
    let baseURL: URL
    var runtimeAuthorityId: UUID?
    var runtimeAuthorityEpoch: Int?
    var endpointId: UUID?
    var friendlyName: String
    let credentialReference: String?
    var trustMode: TrustMode
    var tlsSPKISha256: Data?

    init(
        id: UUID,
        mode: Mode,
        baseURL: URL,
        runtimeAuthorityId: UUID?,
        runtimeAuthorityEpoch: Int? = nil,
        endpointId: UUID? = nil,
        friendlyName: String,
        credentialReference: String?,
        trustMode: TrustMode,
        tlsSPKISha256: Data? = nil
    ) {
        self.id = id
        self.mode = mode
        self.baseURL = baseURL
        self.runtimeAuthorityId = runtimeAuthorityId
        self.runtimeAuthorityEpoch = runtimeAuthorityEpoch
        self.endpointId = endpointId
        self.friendlyName = friendlyName
        self.credentialReference = credentialReference
        self.trustMode = trustMode
        self.tlsSPKISha256 = tlsSPKISha256
    }

    func daemonEndpoint(credential: String?) -> DaemonEndpoint? {
        switch self.mode {
        case .managedLocal:
            return .managedLocal(bearerToken: credential)
        case .attachedLocal:
            guard let credential, !credential.isEmpty, self.trustMode == .loopback else {
                return nil
            }
            return .attachedLocal(baseURL: self.baseURL, bearerToken: credential)
        case .remote:
            guard let credential, !credential.isEmpty,
                  self.trustMode == .pinnedSPKI,
                  let pin = self.tlsSPKISha256,
                  pin.count == 32
            else { return nil }
            return .remote(
                baseURL: self.baseURL,
                bearerToken: credential,
                tlsSPKISha256: pin
            )
        }
    }

    static var managedLocal: Self {
        Self(
            id: UUID(),
            mode: .managedLocal,
            baseURL: URL(string: "http://127.0.0.1:\(DaemonConfig.managedPort)")
                ?? URL(fileURLWithPath: "/"),
            runtimeAuthorityId: nil,
            endpointId: SurfaceIdentity.current(),
            friendlyName: "This Mac",
            credentialReference: nil,
            trustMode: .loopback
        )
    }
}

struct EndpointCredentialConfiguration: Sendable, Equatable {
    static let infoPlistKey = "BoBeEndpointCredentialAccessGroup"
    static let service = "com.bobe.app.endpoint-credential"

    let accessGroup: String

    static func validate(
        bundleIdentifier: String?,
        configuredAccessGroup: String?,
        applicationIdentifier: String?,
        entitledAccessGroups: [String]?
    ) throws -> Self {
        guard let bundleIdentifier, !bundleIdentifier.isEmpty,
              let configuredAccessGroup, !configuredAccessGroup.isEmpty,
              !configuredAccessGroup.contains("$("),
              let applicationIdentifier, !applicationIdentifier.isEmpty,
              let prefix = applicationIdentifier.removingSuffix(bundleIdentifier),
              !prefix.isEmpty,
              configuredAccessGroup == prefix + Self.service,
              let entitledAccessGroups,
              entitledAccessGroups.contains(configuredAccessGroup)
        else {
            throw EndpointCredentialError.invalidApplicationIdentity
        }
        return Self(accessGroup: configuredAccessGroup)
    }

    func baseQuery(reference: String) -> [String: Any] {
        let context = LAContext()
        context.interactionNotAllowed = true
        return [
            kSecClass as String: kSecClassGenericPassword,
            kSecUseDataProtectionKeychain as String: true,
            kSecAttrAccessGroup as String: self.accessGroup,
            kSecAttrService as String: Self.service,
            kSecAttrAccount as String: reference,
            kSecAttrSynchronizable as String: false,
            kSecUseAuthenticationContext as String: context,
        ]
    }
}

private extension String {
    func removingSuffix(_ suffix: String) -> String? {
        guard self.hasSuffix(suffix) else { return nil }
        return String(self.dropLast(suffix.count))
    }
}

final class EndpointCredentialMemory: Sendable {
    static let shared = EndpointCredentialMemory()

    private let credentials = OSAllocatedUnfairLock(initialState: [String: String]())

    func load(reference: String) -> String? {
        self.credentials.withLock { $0[reference] }
    }

    func save(_ value: String, reference: String) {
        self.credentials.withLock { $0[reference] = value }
    }

    func delete(reference: String) {
        self.credentials.withLock { $0[reference] = nil }
    }
}

enum EndpointCredentialBacking: Sendable {
    case keychain(EndpointCredentialConfiguration)
    case processMemory(EndpointCredentialMemory)

    static func select(
        bundleIdentifier: String?,
        configuredAccessGroup: String?,
        applicationIdentifier: String?,
        entitledAccessGroups: [String]?,
        memory: EndpointCredentialMemory = .shared
    ) throws -> Self {
        if applicationIdentifier == nil, entitledAccessGroups == nil {
            return .processMemory(memory)
        }
        return try .keychain(
            EndpointCredentialConfiguration.validate(
                bundleIdentifier: bundleIdentifier,
                configuredAccessGroup: configuredAccessGroup,
                applicationIdentifier: applicationIdentifier,
                entitledAccessGroups: entitledAccessGroups
            )
        )
    }

    static func current(bundle: Bundle = .main) throws -> Self {
        guard let task = SecTaskCreateFromSelf(nil) else {
            throw EndpointCredentialError.invalidApplicationIdentity
        }
        let applicationIdentifier = SecTaskCopyValueForEntitlement(
            task,
            "com.apple.application-identifier" as CFString,
            nil
        ) as? String
        let accessGroups = SecTaskCopyValueForEntitlement(
            task,
            "keychain-access-groups" as CFString,
            nil
        ) as? [String]
        return try Self.select(
            bundleIdentifier: bundle.bundleIdentifier,
            configuredAccessGroup: bundle.object(
                forInfoDictionaryKey: EndpointCredentialConfiguration.infoPlistKey
            ) as? String,
            applicationIdentifier: applicationIdentifier,
            entitledAccessGroups: accessGroups
        )
    }
}

struct EndpointCredentialStore: Sendable {
    private let backing: @Sendable () throws -> EndpointCredentialBacking

    init(
        backing: @escaping @Sendable () throws -> EndpointCredentialBacking = {
            try EndpointCredentialBacking.current()
        }
    ) {
        self.backing = backing
    }

    func load(reference: String) -> String? {
        do {
            switch try self.backing() {
            case let .keychain(configuration):
                return try Self.loadFromKeychain(configuration, reference: reference)
            case let .processMemory(memory):
                return memory.load(reference: reference)
            }
        } catch {
            endpointLogger.error(
                "Endpoint credential read failed: \(error.localizedDescription, privacy: .public)"
            )
            return nil
        }
    }

    func save(_ value: String, reference: String) throws {
        do {
            switch try self.backing() {
            case let .keychain(configuration):
                try Self.saveToKeychain(configuration, value: value, reference: reference)
            case let .processMemory(memory):
                memory.save(value, reference: reference)
            }
        } catch {
            endpointLogger.error(
                "Endpoint credential save failed: \(error.localizedDescription, privacy: .public)"
            )
            throw error
        }
    }

    func delete(reference: String) {
        do {
            switch try self.backing() {
            case let .keychain(configuration):
                let status = SecItemDelete(configuration.baseQuery(reference: reference) as CFDictionary)
                guard status != errSecItemNotFound else { return }
                try EndpointCredentialError.check(status)
            case let .processMemory(memory):
                memory.delete(reference: reference)
            }
        } catch {
            endpointLogger.error(
                "Endpoint credential delete failed: \(error.localizedDescription, privacy: .public)"
            )
        }
    }

    private static func loadFromKeychain(
        _ configuration: EndpointCredentialConfiguration,
        reference: String
    ) throws -> String? {
        var query = configuration.baseQuery(reference: reference)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound {
            return nil
        }
        try EndpointCredentialError.check(status)
        guard let data = result as? Data,
              let credential = String(data: data, encoding: .utf8)
        else {
            throw EndpointCredentialError.invalidCredentialData
        }
        return credential
    }

    private static func saveToKeychain(
        _ configuration: EndpointCredentialConfiguration,
        value: String,
        reference: String
    ) throws {
        let key = configuration.baseQuery(reference: reference)
        let attributes: [String: Any] = [
            kSecValueData as String: Data(value.utf8),
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            kSecAttrSynchronizable as String: false,
        ]
        let update = SecItemUpdate(key as CFDictionary, attributes as CFDictionary)
        guard update == errSecItemNotFound else {
            try EndpointCredentialError.check(update)
            return
        }
        var insert = key
        attributes.forEach { insert[$0.key] = $0.value }
        try EndpointCredentialError.check(SecItemAdd(insert as CFDictionary, nil))
    }
}

enum EndpointStoreError: LocalizedError {
    case profilePersistenceFailed

    var errorDescription: String? {
        L10n.tr("setup.runtime.error.profile_save_failed")
    }
}

enum EndpointCredentialError: LocalizedError, Equatable {
    case invalidApplicationIdentity
    case invalidCredentialData
    case missingEntitlement
    case interactionRequired
    case status(OSStatus)

    static func check(_ status: OSStatus) throws {
        switch status {
        case errSecSuccess:
            return
        case errSecMissingEntitlement:
            throw Self.missingEntitlement
        case errSecInteractionNotAllowed, errSecAuthFailed, errSecUserCanceled:
            throw Self.interactionRequired
        default:
            throw Self.status(status)
        }
    }

    var errorDescription: String? {
        switch self {
        case .invalidApplicationIdentity:
            "BoBe is not signed for its Data Protection credential group."
        case .invalidCredentialData:
            "The BoBe connection credential is not valid UTF-8."
        case .missingEntitlement:
            "BoBe is not entitled to its Data Protection credential group."
        case .interactionRequired:
            "BoBe refused a credential operation that requested user authentication."
        case let .status(status):
            "Could not access the BoBe Data Protection credential (\(status))."
        }
    }
}

struct EndpointSelection: Sendable, Equatable {
    let generation: UInt64
    let profileID: UUID?
    let authorityID: UUID?
    let endpoint: DaemonEndpoint
}

final class EndpointStore: @unchecked Sendable {
    static let shared = EndpointStore()

    private static let profileKey = "bobe.endpoint_profile"
    private let lock = OSAllocatedUnfairLock(
        initialState: Resolved(profile: nil, endpoint: nil)
    )
    private let credentials = EndpointCredentialStore()
    private let mutationLock = NSLock()
    private let defaults: UserDefaults

    private struct Resolved: Sendable {
        var profile: EndpointProfile?
        var endpoint: DaemonEndpoint?
        var generation: UInt64 = 0
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        self.reload()
    }

    var profile: EndpointProfile? {
        self.lock.withLock { $0.profile }
    }

    var runtimeAuthorityId: UUID? {
        self.lock.withLock { $0.profile?.runtimeAuthorityId }
    }

    var endpoint: DaemonEndpoint? {
        self.lock.withLock { $0.endpoint }
    }

    var selection: EndpointSelection {
        self.lock.withLock {
            EndpointSelection(
                generation: $0.generation,
                profileID: $0.profile?.id,
                authorityID: $0.profile?.runtimeAuthorityId,
                endpoint: $0.endpoint ?? .unavailable
            )
        }
    }

    func profile(for selection: EndpointSelection) -> EndpointProfile? {
        self.lock.withLock { state in
            guard state.generation == selection.generation,
                  state.profile?.id == selection.profileID,
                  state.profile?.runtimeAuthorityId == selection.authorityID,
                  (state.endpoint ?? .unavailable) == selection.endpoint
            else { return nil }
            return state.profile
        }
    }

    func credential(for profile: EndpointProfile) -> String? {
        profile.credentialReference.flatMap(self.credentials.load)
    }

    private func commitProfile(_ encoded: Data) throws {
        self.defaults.set(encoded, forKey: Self.profileKey)
        guard self.defaults.data(forKey: Self.profileKey) == encoded else {
            throw EndpointStoreError.profilePersistenceFailed
        }
    }

    func persist(
        _ profile: EndpointProfile,
        credential: String?,
        deletePreviousCredential: Bool = true
    ) throws {
        self.mutationLock.lock()
        defer { self.mutationLock.unlock() }
        let previousReference = self.profile?.credentialReference
        let previousCredential = previousReference.flatMap(self.credentials.load)
        let encoded = try JSONEncoder().encode(profile)
        let newReference = profile.credentialReference
        let savedCredential = newReference != nil && credential != nil
        if let reference = newReference, let credential {
            try self.credentials.save(credential, reference: reference)
        }
        do {
            try self.commitProfile(encoded)
        } catch {
            if savedCredential, let newReference {
                if newReference == previousReference, let previousCredential {
                    try? self.credentials.save(previousCredential, reference: newReference)
                } else {
                    self.credentials.delete(reference: newReference)
                }
            }
            throw error
        }
        if deletePreviousCredential,
           let previousReference,
           previousReference != newReference {
            self.credentials.delete(reference: previousReference)
        }
        self.reload()
        DaemonTransportPolicy.invalidateAll()
    }

    func deleteCredential(reference: String?) {
        if let reference {
            self.credentials.delete(reference: reference)
        }
    }

    func updateNegotiatedIdentity(
        profileID: UUID,
        authorityID: UUID,
        authorityEpoch: Int,
        expectedSelection: EndpointSelection? = nil
    ) throws {
        self.mutationLock.lock()
        defer { self.mutationLock.unlock() }
        if let expectedSelection, self.selection != expectedSelection { throw CancellationError() }
        let previousProfile = self.profile
        guard var current = previousProfile, current.id == profileID else { return }
        current.runtimeAuthorityId = authorityID
        current.runtimeAuthorityEpoch = authorityEpoch
        try self.commitProfile(try JSONEncoder().encode(current))
        self.reload()
        if previousProfile != current {
            DaemonTransportPolicy.invalidateAll()
        }
    }

    func clear() {
        self.mutationLock.lock()
        defer { self.mutationLock.unlock() }
        if let reference = self.profile?.credentialReference {
            self.credentials.delete(reference: reference)
        }
        self.defaults.removeObject(forKey: Self.profileKey)
        self.reload()
        DaemonTransportPolicy.invalidateAll()
    }

    private func reload() {
        let profile = self.defaults.data(forKey: Self.profileKey)
            .flatMap { try? JSONDecoder().decode(EndpointProfile.self, from: $0) }
        let endpoint = profile.flatMap(self.resolve)
        let changed = self.lock.withLock { state in
            let changed = state.profile?.id != profile?.id
                || state.profile?.runtimeAuthorityId != profile?.runtimeAuthorityId
                || state.profile?.runtimeAuthorityEpoch != profile?.runtimeAuthorityEpoch
                || state.endpoint != endpoint
            if changed { state.generation &+= 1 }
            state.profile = profile
            state.endpoint = endpoint
            return changed
        }
        guard changed else { return }
        DispatchQueue.main.async {
            NotificationCenter.default.post(name: .bobeEndpointIdentityChanged, object: nil)
        }
    }

    private func resolve(_ profile: EndpointProfile) -> DaemonEndpoint? {
        profile.daemonEndpoint(credential: self.credential(for: profile))
    }
}
