@testable import BoBe
import LocalAuthentication
import Security
import Testing

@Suite("Endpoint credential configuration")
struct EndpointCredentialConfigurationTests {
    private let bundleID = "com.bobe.app"
    private let applicationID = "TEAM123.com.bobe.app"
    private let accessGroup = "TEAM123.com.bobe.app.endpoint-credential"

    @Test("accepts the one profile-expanded endpoint group")
    func acceptsMatchingIdentity() throws {
        let configuration = try EndpointCredentialConfiguration.validate(
            bundleIdentifier: self.bundleID,
            configuredAccessGroup: self.accessGroup,
            applicationIdentifier: self.applicationID,
            entitledAccessGroups: [self.accessGroup]
        )

        #expect(configuration.accessGroup == self.accessGroup)
    }

    @Test("rejects missing or unexpanded signing material", arguments: [
        nil,
        "$(AppIdentifierPrefix)com.bobe.app.endpoint-credential",
        "OTHER.com.bobe.app.endpoint-credential",
    ])
    func rejectsInvalidConfiguredGroup(configuredGroup: String?) {
        #expect(throws: EndpointCredentialError.invalidApplicationIdentity) {
            try EndpointCredentialConfiguration.validate(
                bundleIdentifier: self.bundleID,
                configuredAccessGroup: configuredGroup,
                applicationIdentifier: self.applicationID,
                entitledAccessGroups: [self.accessGroup]
            )
        }
    }

    @Test("rejects a group absent from effective entitlements")
    func rejectsMissingEntitlement() {
        #expect(throws: EndpointCredentialError.invalidApplicationIdentity) {
            try EndpointCredentialConfiguration.validate(
                bundleIdentifier: self.bundleID,
                configuredAccessGroup: self.accessGroup,
                applicationIdentifier: self.applicationID,
                entitledAccessGroups: []
            )
        }
    }

    @Test("every operation query is Data Protection-only and non-synchronizing")
    func queryInvariants() throws {
        let configuration = try EndpointCredentialConfiguration.validate(
            bundleIdentifier: self.bundleID,
            configuredAccessGroup: self.accessGroup,
            applicationIdentifier: self.applicationID,
            entitledAccessGroups: [self.accessGroup]
        )
        let query = configuration.baseQuery(reference: "opaque-reference")

        #expect(query[kSecClass as String] as? String == kSecClassGenericPassword as String)
        #expect(query[kSecUseDataProtectionKeychain as String] as? Bool == true)
        #expect(query[kSecAttrAccessGroup as String] as? String == self.accessGroup)
        #expect(query[kSecAttrService as String] as? String == EndpointCredentialConfiguration.service)
        #expect(query[kSecAttrAccount as String] as? String == "opaque-reference")
        let context = query[kSecUseAuthenticationContext as String] as? LAContext
        #expect(query[kSecAttrSynchronizable as String] as? Bool == false)
        #expect(context?.interactionNotAllowed == true)
    }

    @Test("a build with any signing identity uses the Keychain or fails", arguments: [
        ("TEAM123.com.bobe.app", ["TEAM123.com.bobe.app.endpoint-credential"], true),
        ("TEAM123.com.bobe.app", [], false),
        ("TEAM123.com.bobe.app", nil, false),
        (nil, ["TEAM123.com.bobe.app.endpoint-credential"], false),
    ] as [(String?, [String]?, Bool)])
    func signedBuildsUseOnlyTheKeychain(applicationID: String?, groups: [String]?, valid: Bool) throws {
        let select = {
            try EndpointCredentialBacking.select(
                bundleIdentifier: self.bundleID,
                configuredAccessGroup: self.accessGroup,
                applicationIdentifier: applicationID,
                entitledAccessGroups: groups
            )
        }
        guard valid else {
            #expect(throws: EndpointCredentialError.invalidApplicationIdentity) { try select() }
            return
        }
        guard case let .keychain(configuration) = try select() else {
            Issue.record("a provisioned build did not select the Keychain")
            return
        }
        #expect(configuration.accessGroup == self.accessGroup)
    }

    @Test("a build without a signing identity keeps credentials only while it runs")
    func unsignedBuildsKeepCredentialsInMemory() throws {
        let memory = EndpointCredentialMemory()
        let backing = try EndpointCredentialBacking.select(
            bundleIdentifier: self.bundleID,
            configuredAccessGroup: "$(AppIdentifierPrefix)com.bobe.app.endpoint-credential",
            applicationIdentifier: nil,
            entitledAccessGroups: nil,
            memory: memory
        )
        guard case .processMemory = backing else {
            Issue.record("an unsigned build did not select process memory")
            return
        }
        let store = EndpointCredentialStore { backing }

        #expect(store.load(reference: "reference") == nil)
        try store.save("first", reference: "reference")
        try store.save("second", reference: "reference")
        #expect(store.load(reference: "reference") == "second")
        #expect(EndpointCredentialStore { .processMemory(EndpointCredentialMemory()) }.load(reference: "reference") == nil)
        store.delete(reference: "reference")
        #expect(store.load(reference: "reference") == nil)
    }

    @Test("maps entitlement and interaction statuses without fallback")
    func statusMapping() {
        #expect(throws: EndpointCredentialError.missingEntitlement) {
            try EndpointCredentialError.check(errSecMissingEntitlement)
        }
        #expect(throws: EndpointCredentialError.interactionRequired) {
            try EndpointCredentialError.check(errSecInteractionNotAllowed)
        }
        #expect(throws: EndpointCredentialError.status(-1)) {
            try EndpointCredentialError.check(-1)
        }
    }
}
