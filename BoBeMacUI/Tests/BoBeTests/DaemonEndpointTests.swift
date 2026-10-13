@testable import BoBe
import Foundation
import Testing

@Suite("Daemon endpoint")
struct DaemonEndpointTests {
    @Test
    func managedLocalOverridesRequireExplicitDevMode() {
        let home = URL(fileURLWithPath: "/Users/test", isDirectory: true)
        let production = [
            "BOBE_MANAGED_PORT": "18767",
            "BOBE_MANAGED_DATA_DIR": "/tmp/bobe-isolated",
        ]
        #expect(DaemonConfig.managedPort(environment: production) == DaemonConfig.defaultPort)
        #expect(
            DaemonConfig.managedDataDirectory(environment: production, homeDirectory: home).path
                == "/Users/test/.bobe"
        )

        let development = production.merging(["BOBE_DEV": "1"]) { _, new in new }
        #expect(DaemonConfig.managedPort(environment: development) == 18767)
        #expect(
            DaemonConfig.managedDataDirectory(environment: development, homeDirectory: home).path
                == "/tmp/bobe-isolated"
        )
    }

    @Test
    func managedLocalOverridesRejectUnsafeValues() {
        let home = URL(fileURLWithPath: "/Users/test", isDirectory: true)
        for port in ["0", "65536", "not-a-port"] {
            #expect(
                DaemonConfig.managedPort(environment: [
                    "BOBE_DEV": "1",
                    "BOBE_MANAGED_PORT": port,
                ]) == DaemonConfig.defaultPort
            )
        }
        #expect(
            DaemonConfig.managedDataDirectory(
                environment: [
                    "BOBE_DEV": "1",
                    "BOBE_MANAGED_DATA_DIR": "relative/path",
                ],
                homeDirectory: home
            ).path == "/Users/test/.bobe"
        )
    }

    @Test
    func profilesResolveOnlyThroughTheirTrustedTransport() throws {
        let loopback = try #require(URL(string: "http://127.0.0.1:8766"))
        let remote = try #require(URL(string: "https://bobe.example.test"))
        let pin = Data(repeating: 7, count: 32)

        let attached = EndpointProfile(
            id: UUID(), mode: .attachedLocal, baseURL: loopback,
            runtimeAuthorityId: nil, friendlyName: "Local BoBe",
            credentialReference: "local", trustMode: .loopback
        )
        #expect(attached.daemonEndpoint(credential: "credential")?.isLoopbackRuntime == true)
        #expect(attached.daemonEndpoint(credential: nil) == nil)

        let pinned = EndpointProfile(
            id: UUID(), mode: .remote, baseURL: remote,
            runtimeAuthorityId: UUID(), friendlyName: "Remote BoBe",
            credentialReference: "remote", trustMode: .pinnedSPKI,
            tlsSPKISha256: pin
        )
        #expect(pinned.daemonEndpoint(credential: "credential")?.tlsSPKISha256 == pin)

        let missingPin = EndpointProfile(
            id: UUID(), mode: .remote, baseURL: remote,
            runtimeAuthorityId: UUID(), friendlyName: "Remote BoBe",
            credentialReference: "remote", trustMode: .pinnedSPKI
        )
        #expect(missingPin.daemonEndpoint(credential: "credential") == nil)
    }

    @Test
    func remoteEndpointAddsBearerAuthorization() throws {
        let url = try #require(URL(string: "https://bobe.example.test"))
        let endpoint = DaemonEndpoint.remote(baseURL: url, bearerToken: "secret-token")
        var request = URLRequest(url: url)

        endpoint.authorize(&request)

        #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer secret-token")
    }

    @Test
    func attachedLocalCanUseOptionalBearerWithoutOwningLifecycle() throws {
        let url = try #require(URL(string: "http://127.0.0.1:8766"))
        let endpoint = DaemonEndpoint.attachedLocal(baseURL: url, bearerToken: "local-token")
        var request = URLRequest(url: url)
        endpoint.authorize(&request)
        #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer local-token")
        #expect(!endpoint.managesLocalProcess)
        #expect(!endpoint.canRestartRuntime)
    }

    @Test
    func storedProfileIsTheOnlyConfiguredEndpointSource() throws {
        let defaults = try #require(EphemeralUserDefaults(prefix: "DaemonEndpointTests"))
        let profile = EndpointProfile.managedLocal
        defaults.set(try JSONEncoder().encode(profile), forKey: "bobe.endpoint_profile")

        let store = EndpointStore(defaults: defaults)

        #expect(store.profile?.id == profile.id)
        #expect(store.endpoint?.managesLocalProcess == true)
    }

    @Test
    func endpointSelectionRejectsEarlierVisitToSameAuthority() throws {
        let suiteName = "DaemonEndpointTests.selection.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let store = EndpointStore(defaults: defaults)
        let original = EndpointProfile.managedLocal
        try store.persist(original, credential: nil)
        let first = store.selection
        try store.persist(.managedLocal, credential: nil)
        try store.persist(original, credential: nil)
        #expect(store.selection.profileID == first.profileID)
        #expect(store.selection.generation > first.generation)
        #expect(store.selection != first)
    }

    @Test
    func delayedAttachCannotUpdateRevisitedProfile() throws {
        let suiteName = "DaemonEndpointTests.attach-generation.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let store = EndpointStore(defaults: defaults)
        var profile = EndpointProfile.managedLocal
        profile.runtimeAuthorityId = UUID()
        profile.runtimeAuthorityEpoch = 3
        try store.persist(profile, credential: nil)
        let oldSelection = store.selection
        try store.persist(.managedLocal, credential: nil)
        try store.persist(profile, credential: nil)
        #expect(store.profile(for: oldSelection) == nil)
        #expect(throws: CancellationError.self) {
            try store.updateNegotiatedIdentity(
                profileID: profile.id, authorityID: UUID(), authorityEpoch: 1,
                expectedSelection: oldSelection
            )
        }
        #expect(store.profile?.runtimeAuthorityId == profile.runtimeAuthorityId)
        #expect(store.profile?.runtimeAuthorityEpoch == 3)
    }

    @Test
    func outOfOrderEventStreamCommandsCannotSupersedeNewerIntent() {
        var order = EventStreamCommandOrder()
        let accepted = [2, 1, 2, 4, 3].map { order.accept(UInt64($0)) }
        #expect(accepted == [true, false, false, true, false])
        #expect(order.latest == 4)
    }

    @Test
    func boundClientKeepsItsOriginalTransport() async throws {
        let endpoint = DaemonEndpoint.remote(
            baseURL: try #require(URL(string: "https://original.example.test")),
            bearerToken: "original-credential", tlsSPKISha256: Data(repeating: 1, count: 32)
        )
        let client = DaemonClient(endpoint: endpoint)
        #expect(await client.endpoint == endpoint)
        #expect(await client.endpointURL("settings").host == "original.example.test")
    }

    @Test
    func attachResponseDecodesVersionAndCapabilities() throws {
        let data = try #require(
            """
            {"runtime_authority_id":"11111111-2222-3333-4444-555555555555","runtime_authority_epoch":1,"runtime_incarnation_id":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee","daemon_version":"0.1.0","schema_version":1,"started_at":"2026-07-29T00:00:00Z","protocol_min":1,"protocol_max":1,"negotiated_protocol":1,"deployment_mode":"loopback","initialization_state":"ready","readiness":{"state":"ready","blocking_reasons":[]},"capabilities":{"agent":"ready","stt":"unsupported","tts":"degraded","capture":"ready","body":"unsupported","mcp":"ready","events":"ready"},"granted_scopes":["conversation_read"],"auth_mode":"owner_token","events":{"subscription":"resumable_broadcast","cursor_start":null}}
            """.data(using: .utf8)
        )
        let response = try JSONDecoder().decode(HelloResponse.self, from: data)
        #expect(response.negotiatedProtocol == 1)
        #expect(response.capabilities.stt == .unsupported)
        #expect(response.initializationState == .ready)
    }

    @Test
    func readinessResponseDecodesDaemonIdentityAndInitialization() throws {
        let data = try #require(
            """
            {"status":"ready","runtime_authority_id":"11111111-2222-3333-4444-555555555555","runtime_authority_epoch":1,"runtime_incarnation_id":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee","daemon_version":"0.1.0","schema_version":1,"initialization_state":"provider_required","started_at":"2026-07-29T00:00:00Z","blocking_reasons":[]}
            """.data(using: .utf8)
        )
        let response = try JSONDecoder().decode(ReadinessResponse.self, from: data)
        #expect(response.initializationState == .providerRequired)
        #expect(response.runtimeAuthorityEpoch == 1)
    }

    @Test
    func providerResponseDecodesRoutesAndCredentialPresence() throws {
        let json = """
        {
          "profiles": [{
            "id": "openai", "kind": "open_ai", "auth_method": "api_key", "base_url": null,
            "organization": null, "local_runtime": null,
            "models": {"chat": null, "goals": null, "consolidate": null, "small": null},
            "offline": false, "credential_present": true, "restart_required": false
          }, {
            "id": "anthropic", "kind": "anthropic", "auth_method": "api_key", "base_url": null,
            "organization": null, "local_runtime": null,
            "models": {"chat": null, "goals": null, "consolidate": null, "small": null},
            "offline": false, "credential_present": false, "restart_required": false
          }],
          "routes": {
            "chat": {"profile":"openai","model":"gpt-5","reasoning_effort":null},
            "goals": {"profile":"openai","model":"gpt-5-mini","reasoning_effort":null},
            "consolidate": {"profile":"openai","model":"gpt-5-mini","reasoning_effort":null}
          },
          "active_profiles": [],
          "active_routes": {
            "chat": {"profile":"openai","model":"gpt-5","reasoning_effort":null},
            "goals": {"profile":"openai","model":"gpt-5-mini","reasoning_effort":null},
            "consolidate": {"profile":"openai","model":"gpt-5-mini","reasoning_effort":null}
          },
          "restart_required": false
        }
        """
        let response = try JSONDecoder().decode(ProvidersResponse.self, from: Data(json.utf8))
        #expect(response.profiles.first?.kind == ProviderKindWire.openAI)
        #expect(response.profiles.first?.credentialPresent == true)
        #expect(response.profiles.last?.kind == ProviderKindWire.anthropic)
        #expect(response.routes.chat.model == "gpt-5")
    }

    @Test
    func localMacEndpointsRequestCaptureLeaseButRemoteDoesNot() throws {
        let attached = DaemonEndpoint.attachedLocal(
            baseURL: try #require(URL(string: "http://127.0.0.1:9876")),
            bearerToken: nil
        )
        let remote = DaemonEndpoint.remote(
            baseURL: try #require(URL(string: "https://bobe.example.com")),
            bearerToken: "secret"
        )

        #expect(DaemonEndpoint.managedLocal(bearerToken: nil).daemonFilesystemIsLocalToApp)
        #expect(DaemonEndpoint.managedLocal(bearerToken: nil).isLoopbackRuntime)
        #expect(!attached.daemonFilesystemIsLocalToApp)
        #expect(attached.isLoopbackRuntime)
        #expect(!remote.daemonFilesystemIsLocalToApp)
        #expect(!remote.isLoopbackRuntime)
    }

    @Test
    func managedLocalEndpointDoesNotAddAuthorization() throws {
        let url = try #require(URL(string: "http://127.0.0.1:8766"))
        var request = URLRequest(url: url)

        DaemonEndpoint.managedLocal(bearerToken: nil).authorize(&request)

        #expect(request.value(forHTTPHeaderField: "Authorization") == nil)
    }

    @Test
    func queryItemsArePercentEncoded() async {
        let url = await DaemonClient.shared.endpointURL(
            "/models",
            queryItems: [URLQueryItem(name: "engine", value: "local&debug=true")]
        )

        #expect(url.query == "engine=local%26debug%3Dtrue")
    }

    @Test
    func opaquePathSegmentsEncodeReservedCharactersWithoutChangingRouteShape() async throws {
        let baseURL = try #require(URL(string: "https://example.test"))
        let url = DaemonClient.endpointURL(
            baseURL: baseURL,
            pathSegments: ["endpoints", "../surface/100%?# café"]
        )

        let components = try #require(URLComponents(url: url, resolvingAgainstBaseURL: false))
        #expect(url.pathComponents == ["/", "endpoints", "../surface/100%?# café"])
        #expect(components.percentEncodedPath == "/endpoints/..%2Fsurface%2F100%25%3F%23%20caf%C3%A9")
        #expect(url.query == nil)
        #expect(url.fragment == nil)
    }

    @Test
    func pathSegmentsPreserveConfiguredBasePrefix() throws {
        let baseURL = try #require(URL(string: "https://example.test/bobe/api"))
        let result = DaemonClient.endpointURL(
            baseURL: baseURL,
            pathSegments: ["endpoints", "surface/id"]
        )
        let resultComponents = try #require(
            URLComponents(url: result, resolvingAgainstBaseURL: false)
        )
        #expect(resultComponents.percentEncodedPath == "/bobe/api/endpoints/surface%2Fid")
    }

    @Test
    func messageRequestCarriesStableIdempotencyKey() throws {
        let requestId = try #require(UUID(uuidString: "11111111-2222-3333-4444-555555555555"))
        let encoded = try #require(
            JSONSerialization.jsonObject(
                with: JSONEncoder().encode(
                    SendMessageRequest(content: "hello", requestId: requestId)
                )
            ) as? [String: Any]
        )

        #expect(encoded["content"] as? String == "hello")
        #expect(
            (encoded["request_id"] as? String)?.lowercased()
                == requestId.uuidString.lowercased()
        )
    }

    @Test
    func agentRuntimeUsesExplicitDeveloperPaths() {
        let bundleURL = URL(fileURLWithPath: "/Applications/BoBe.app", isDirectory: true)
        let node = "/project/tools/node"
        let entry = "/project/BoBeAgent/dist/main.js"

        let resolved = BackendService.resolveAgentRuntime(
            environment: ["BOBE_NODE_PATH": node, "BOBE_AGENT_ENTRY": entry],
            bundleURL: bundleURL,
            isExecutableFile: { $0 == node },
            isReadableFile: { $0 == entry }
        )

        #expect(resolved == BackendService.AgentRuntimePaths(node: node, entry: entry))
    }

    @Test
    func agentRuntimeUsesBundledNodeAndWorker() {
        let bundleURL = URL(fileURLWithPath: "/Applications/BoBe.app", isDirectory: true)
        let node = "/Applications/BoBe.app/Contents/MacOS/node"
        let entry = "/Applications/BoBe.app/Contents/Resources/agent/dist/main.js"

        let resolved = BackendService.resolveAgentRuntime(
            environment: [:],
            bundleURL: bundleURL,
            isExecutableFile: { $0 == node },
            isReadableFile: { $0 == entry }
        )

        #expect(resolved == BackendService.AgentRuntimePaths(node: node, entry: entry))
    }

    @Test
    func agentRuntimePrefersCompleteBundleOverEnvironment() {
        let bundleURL = URL(fileURLWithPath: "/Applications/BoBe.app", isDirectory: true)
        let node = "/Applications/BoBe.app/Contents/MacOS/node"
        let entry = "/Applications/BoBe.app/Contents/Resources/agent/dist/main.js"

        let resolved = BackendService.resolveAgentRuntime(
            environment: ["BOBE_NODE_PATH": "/other/node", "BOBE_AGENT_ENTRY": "/other/main.js"],
            bundleURL: bundleURL,
            isExecutableFile: { _ in true },
            isReadableFile: { _ in true }
        )

        #expect(resolved == BackendService.AgentRuntimePaths(node: node, entry: entry))
    }

    @Test
    func agentRuntimeRejectsMissingExecutables() {
        let resolved = BackendService.resolveAgentRuntime(
            environment: ["BOBE_NODE_PATH": "/missing/node", "BOBE_AGENT_ENTRY": "/missing/main.js"],
            bundleURL: URL(fileURLWithPath: "/Applications/BoBe.app", isDirectory: true),
            isExecutableFile: { _ in false },
            isReadableFile: { _ in false }
        )

        #expect(resolved == nil)
    }

    @Test
    func agentRuntimeDoesNotMixIncompleteOverridesWithBundleAssets() {
        let resolved = BackendService.resolveAgentRuntime(
            environment: ["BOBE_NODE_PATH": "/project/node"],
            bundleURL: URL(fileURLWithPath: "/Applications/BoBe.app", isDirectory: true),
            isExecutableFile: { $0 == "/project/node" },
            isReadableFile: { $0.hasSuffix("/agent/dist/main.js") }
        )
        #expect(resolved == nil)
    }

    @Test
    func runtimePairingAddressValidationEnforcesSelectedMode() {
        #expect(DevicePairingService.isValidAddress("127.0.0.1:8766", expectedMode: .attachedLocal))
        #expect(DevicePairingService.isValidAddress("http://localhost:8766/", expectedMode: .attachedLocal))
        #expect(!DevicePairingService.isValidAddress("https://127.0.0.1:8766", expectedMode: .attachedLocal))
        #expect(!DevicePairingService.isValidAddress("http://192.168.1.20:8766", expectedMode: .attachedLocal))

        #expect(DevicePairingService.isValidAddress("bobe.example.test", expectedMode: .remote))
        #expect(DevicePairingService.isValidAddress("https://bobe.example.test:9443/", expectedMode: .remote))
        #expect(!DevicePairingService.isValidAddress("http://bobe.example.test", expectedMode: .remote))
        #expect(!DevicePairingService.isValidAddress("https://user@bobe.example.test", expectedMode: .remote))
        #expect(!DevicePairingService.isValidAddress("https://bobe.example.test/api", expectedMode: .remote))
        #expect(!DevicePairingService.isValidAddress("https://bobe.example.test?mode=test", expectedMode: .remote))
        #expect(!DevicePairingService.isValidAddress("https://bobe.example.test", expectedMode: .managedLocal))
    }

    @Test
    @MainActor
    func runtimePairingCodeNormalizationKeepsSixASCIIDigits() {
        #expect(RuntimeLocationStepView.normalizedPairingCode(" 12-34 56 78 ") == "123456")
        #expect(RuntimeLocationStepView.normalizedPairingCode("１２٣٤٥٦") == "")
        #expect(RuntimeLocationStepView.normalizedPairingCode("abc") == "")
    }

    @Test
    @MainActor
    func runtimePairingFormRequiresModeValidAddressAndCompleteCode() {
        #expect(RuntimeLocationStepView.isValidForm(
            address: "http://127.0.0.1:8766",
            code: "123 456",
            mode: .attachedLocal
        ))
        #expect(RuntimeLocationStepView.isValidForm(
            address: "https://bobe.example.test",
            code: "123456",
            mode: .remote
        ))
        #expect(!RuntimeLocationStepView.isValidForm(
            address: "http://bobe.example.test",
            code: "123456",
            mode: .remote
        ))
        #expect(!RuntimeLocationStepView.isValidForm(
            address: "http://127.0.0.1:8766",
            code: "12345",
            mode: .attachedLocal
        ))
    }

    @Test
    func runtimeIssuesChooseOnlyValidRecoveryActions() {
        let unreachable = RuntimeIssue(AttachError.unreachable)
        #expect(unreachable.kind == .unreachable)
        #expect(unreachable.primaryAction == .retryConnection)
        #expect(unreachable.expertDetails != nil)

        let rejected = RuntimeIssue(AttachError.unauthorized)
        #expect(rejected.kind == .credentialRejected)
        #expect(rejected.primaryAction == .pairAgain)
        #expect(rejected.expertDetails == nil)

        let notReady = RuntimeIssue(AttachError.notReady(reasons: ["database_unavailable"]))
        #expect(notReady.kind == .notReady)
        #expect(notReady.primaryAction == .retryConnection)

        let authorityChanged = RuntimeIssue(AttachError.authorityRollback(expected: 4, actual: 3))
        #expect(authorityChanged.kind == .authorityChanged)
        #expect(authorityChanged.primaryAction == .pairAgain)

        let incompatible = RuntimeIssue(AttachError.protocolIncompatible)
        #expect(incompatible.kind == .protocolIncompatible)
        #expect(incompatible.primaryAction == .chooseRuntime)

        let incomplete = RuntimeIssue(AttachError.missingScopes(["memory_write"]))
        #expect(incomplete.kind == .accessIncomplete)
        #expect(incomplete.primaryAction == .pairAgain)

        let localMaterial = RuntimeIssue(EndpointCredentialError.status(errSecAuthFailed))
        #expect(localMaterial.kind == .localMaterial)
        #expect(localMaterial.primaryAction == .pairAgain)

        let managedStartup = RuntimeIssue(BackendServiceError.healthCheckFailed)
        #expect(managedStartup.kind == .managedStartup)
        #expect(managedStartup.primaryAction == .retryConnection)
    }

    @Test
    func runtimeIssuesSeparateSecureVerificationFromOrdinaryOutages() {
        let secure = RuntimeIssue(
            NSError(domain: NSURLErrorDomain, code: NSURLErrorServerCertificateUntrusted)
        )
        #expect(secure.kind == .secureVerificationFailed)
        #expect(secure.primaryAction == .pairAgain)

        let timeout = RuntimeIssue(
            NSError(domain: NSURLErrorDomain, code: NSURLErrorTimedOut)
        )
        #expect(timeout.kind == .unreachable)
        #expect(timeout.primaryAction == .retryConnection)
        #expect(!RuntimeIssue.isSecureVerificationFailure(
            NSError(domain: NSURLErrorDomain, code: NSURLErrorTimedOut)
        ))
    }

    @Test
    func agentRuntimeRejectsRelativeDeveloperOverrides() {
        let resolved = BackendService.resolveAgentRuntime(
            environment: ["BOBE_NODE_PATH": "node", "BOBE_AGENT_ENTRY": "dist/main.js"],
            bundleURL: URL(fileURLWithPath: "/Applications/BoBe.app", isDirectory: true),
            isExecutableFile: { !$0.hasPrefix("/") },
            isReadableFile: { !$0.hasPrefix("/") }
        )
        #expect(resolved == nil)
    }
}
