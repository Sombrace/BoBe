@testable import BoBe
import Foundation
import Testing

@Suite("Provider authentication")
struct ProviderAuthenticationTests {
    @Test
    func durableWorkControlDTOsDecodePausedQuestionsSchedulesAndRuns() throws {
        let question = try JSONDecoder().decode(OwnerInputWire.self, from: Data("""
        {"id":"019a3a19-4410-7000-8000-000000000001","kind":"approval","question":"Send the draft?",
        "action":"Email the reviewed draft to Pat.","prompt":"Email the reviewed draft to Pat. Send it?",
        "choices":[{"value":"approve","label":"Yes"},{"value":"decline","label":"No"}],
        "status":"pending","expires_at":"2026-10-07T10:00:00.000Z","response":null,"resumed_job_id":null}
        """.utf8))
        #expect(question.status == .pending)
        #expect(question.choices.map(\.value) == ["approve", "decline"])
        let schedule = try JSONDecoder().decode(AgentScheduleWire.self, from: Data("""
        {"id":"019a3a19-4410-7000-8000-000000000001","title":"Review","task":"Review new evidence",
        "next_run_at":"2026-10-07T10:00:00.000Z","every_seconds":3600,"enabled":true,"generation":1,"last_error":null}
        """.utf8))
        #expect(schedule.nextRunDate != nil)
        #expect(schedule.everySeconds == 3600)
        let run = try JSONDecoder().decode(AgentRunWire.self, from: Data("""
        {"id":"019a3a19-4410-7000-8000-000000000001","kind":"user","stopping":false,"queued":true}
        """.utf8))
        #expect(run.queued)
        let waiting = try JSONDecoder().decode(AgentJobWire.self, from: Data("""
        {"id":"019a3a19-4410-7000-8000-000000000001","title":"Draft","task":"Send draft","status":"awaiting_input",
        "attempt":1,"output":null,"error":null,"delivery_status":"committed"}
        """.utf8))
        #expect(waiting.canCancel)
        #expect(!waiting.canResume)
    }

    @Test
    func inactiveProviderRetainsItsOwnWorkloadModels() throws {
        let response = try JSONDecoder().decode(ProvidersResponse.self, from: Data("""
        {"profiles":[{"id":"relay","kind":"rail_relay","auth_method":"api_key","base_url":"https://relay.example/v1","offline":false,"credential_present":true,"restart_required":false,
        "models":{"chat":"relay-chat","goals":"relay-work","small":"relay-small","consolidate":"relay-memory"}}],
        "routes":{"chat":{"profile":"local","model":"local-chat"},"goals":{"profile":"local","model":"local-work"},
        "consolidate":{"profile":"local","model":"local-memory"},
        "small":{"profile":"local","model":"local-small"}},
        "active_profiles":[],"active_routes":{"chat":{"profile":"local","model":"local-chat"},"goals":{"profile":"local","model":"local-work"},
        "consolidate":{"profile":"local","model":"local-memory"}},"restart_required":false}
        """.utf8))
        var fields = ProviderConfigurationFields()
        fields.load(response, profileID: "relay")
        #expect(fields.chat == "relay-chat")
        #expect(fields.batch == "relay-work")
        #expect(fields.small == "relay-small")
        #expect(fields.consolidate == "relay-memory")
    }

    @Test
    func jobAndToolStateKeepDesiredAndActiveCapabilitiesDistinct() throws {
        let selection = try JSONDecoder().decode(AgentToolSelection.self, from: Data("""
        {"tools":[{"name":"bash","description":"Shell","enabled":false,"active":true}],"restart_required":true}
        """.utf8))
        #expect(selection.restartRequired)
        #expect(selection.tools.first?.enabled == false)
        #expect(selection.tools.first?.active == true)
        let job = try JSONDecoder().decode(AgentJobWire.self, from: Data("""
        {"id":"019a3a19-4410-7000-8000-000000000001","title":"Interrupted work","task":"Inspect a file","status":"interrupted","attempt":1,"output":null,"error":"Inspect effects before resuming","delivery_status":"pending"}
        """.utf8))
        #expect(job.canResume)
        #expect(!job.canCancel)
        let delivering = try JSONDecoder().decode(AgentJobWire.self, from: Data("""
        {"id":"019a3a19-4410-7000-8000-000000000001","title":"Failed work","task":"Inspect a file","status":"failed","attempt":1,"output":null,"error":"Failure","delivery_status":"delivering"}
        """.utf8))
        #expect(!delivering.canResume)
    }
    @Test(arguments: ProviderKindWire.allCases)
    func profilesDecodeTheirAuthMethodAndRequestsSendTheDefault(_ kind: ProviderKindWire) throws {
        let data = Data("""
        {"id":"profile","kind":"\(kind.rawValue)","auth_method":"\(kind.defaultAuthMethod.rawValue)","offline":false,"credential_present":false,"restart_required":false,
        "models":{"chat":null,"goals":null,"consolidate":null,"small":null}}
        """.utf8)
        let profile = try JSONDecoder().decode(ProviderProfileWire.self, from: data)
        #expect(profile.authMethod == kind.defaultAuthMethod)
        let request = ProviderActivationRequest(kind: kind)
        let upsert = ProviderUpsertRequest(request)
        let encoded = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(upsert)) as? [String: Any])
        #expect(encoded["kind"] as? String == kind.rawValue)
        #expect(encoded["auth_method"] as? String == kind.defaultAuthMethod.rawValue)
    }

    @Test
    func oauthActivationDoesNotSendAnAPIKey() throws {
        var fields = ProviderConfigurationFields()
        fields.baseURL = "https://api.example/v1"
        fields.organization = "organization"
        fields.selectAuthMethod(.oauth)
        fields.chat = "subscription-model"
        #expect(!fields.canActivate(apiKey: "not-an-oauth-token"))
        fields.isAuthenticated = true
        #expect(fields.canActivate(apiKey: "not-an-oauth-token"))
        let request = fields.activation(apiKey: "not-an-oauth-token")
        #expect(request.authMethod == .oauth)
        #expect(request.apiKey == nil)
        #expect(request.baseURL == nil)
        #expect(request.organization == nil)
        #expect(fields.profileID == "openai-oauth")
        let encoded = try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any])
        #expect(encoded["auth_method"] as? String == "oauth")
        #expect(encoded["api_key"] == nil)
    }

    @Test
    func providerActivationFillsUnchosenWorkloadsFromTheChatModel() {
        var fields = ProviderConfigurationFields(kind: .copilot)
        fields.isAuthenticated = true
        #expect(!fields.canActivate(apiKey: ""))
        fields.chat = "selected-model"
        #expect(fields.canActivate(apiKey: ""))
        let request = fields.activation(apiKey: "")
        #expect(request.goalsModel == "selected-model")
        #expect(request.consolidateModel == "selected-model")
        #expect(request.smallModel == "selected-model")
        fields.batch = "background-model"
        fields.small = "decision-model"
        fields.consolidate = "memory-model"
        #expect(fields.activation(apiKey: "").goalsModel == "background-model")
        #expect(fields.activation(apiKey: "").smallModel == "decision-model")
        #expect(fields.activation(apiKey: "").consolidateModel == "memory-model")
    }

    @Test
    func relayRequiresTokenAndEndpointButCompatibleLocalNeedsNoKey() {
        var relay = ProviderConfigurationFields(kind: .railRelay)
        relay.chat = "model-id"
        #expect(!relay.canActivate(apiKey: "relay-token"))
        relay.baseURL = "https://relay.example/v1"
        #expect(!relay.canActivate(apiKey: ""))
        #expect(relay.canActivate(apiKey: "relay-token"))
        #expect(relay.activation(apiKey: "relay-token").apiKey == "relay-token")
        #expect(!relay.kind.supportsOAuth)
        relay.selectAuthMethod(.oauth)
        #expect(relay.authMethod == .apiKey)

        var local = ProviderConfigurationFields(kind: .openAICompatible)
        local.baseURL = "http://127.0.0.1:11434/v1"
        local.chat = "local-model"
        #expect(!local.canActivate(apiKey: ""))
        local.offline = true
        #expect(local.canActivate(apiKey: ""))
        local.profileID = "invalid/profile"
        #expect(!local.canActivate(apiKey: ""))
        local.profileID = "local"
        local.baseURL = "https://user:secret@example.com/v1"
        #expect(!local.canActivate(apiKey: ""))
    }

    @Test
    func anthropicDoesNotOfferThirdPartySubscriptionLogin() {
        var fields = ProviderConfigurationFields(kind: .anthropic)
        #expect(!fields.kind.supportsOAuth)
        fields.selectAuthMethod(.oauth)
        #expect(fields.authMethod == .apiKey)
    }

    @Test
    func addingAccessDoesNotReplaceAnExistingProfileOrInventModels() {
        #expect(ProviderConfigurationFields.availableID("openai", existing: ["openai", "openai-2"]) == "openai-3")
        let fields = ProviderConfigurationFields()
        #expect(fields.chat.isEmpty)
        #expect(fields.batch.isEmpty)
    }

    @Test
    func presenceLevelsSetScreenObservationAndCheckinsIndependently() {
        let quiet = ProactivityLevel.quiet.settings
        #expect(!quiet.captureEnabled)
        #expect(!quiet.checkinEnabled)
        let balanced = ProactivityLevel.balanced.settings
        #expect(balanced.captureEnabled)
        #expect(!balanced.checkinEnabled)
        let proactive = ProactivityLevel.proactive.settings
        #expect(proactive.captureEnabled)
        #expect(proactive.checkinEnabled)
    }

    @Test
    func providerLocationUsesTheEndpointNotTheLocalRuntimeInstaller() {
        let local = ProviderProfileWire(id: "custom", kind: .openAICompatible, baseURL: "http://127.0.0.1:8080/v1")
        #expect(local.runsOnAuthority)
        let network = ProviderProfileWire(id: "lan", kind: .openAICompatible, baseURL: "https://models.home/v1", offline: true)
        #expect(!network.runsOnAuthority)
        #expect(network.destination == "models.home")
        #expect(!ProviderProfileWire(id: "relay", kind: .railRelay).runsOnAuthority)
    }

    @Test
    func onlyTheChosenWorkloadIsEncodedIncludingItsReasoning() throws {
        let route = WorkloadRouteWire(profile: "relay", model: "chosen", reasoningEffort: "low")
        let data = try JSONEncoder().encode([ProviderWorkload.goals.rawValue: route])
        let payload = try #require(JSONSerialization.jsonObject(with: data) as? [String: Any])
        #expect(Set(payload.keys) == ["goals"])
        let goal = try #require(payload["goals"] as? [String: Any])
        #expect(goal["reasoning_effort"] as? String == "low")
    }

    @Test(arguments: [false, true])
    func genericLoginDecodesInteractiveInput(_ password: Bool) throws {
        let data = Data("""
        {"phase":"awaiting_input","message":"Paste the authorization response","password":\(password)}
        """.utf8)
        let phase = try JSONDecoder().decode(ProviderLoginPhase.self, from: data)
        #expect(phase == .awaitingInput(message: "Paste the authorization response", password: password))
        #expect(!phase.isTerminal)
    }

    @Test
    func modelsRetainOptionalMetadata() throws {
        let data = Data("""
        {"profile":"relay","models":[{"id":"model","name":"Model","vision":true,
        "context_window":128000,"default_reasoning_effort":"medium",
        "supported_reasoning_efforts":["low","medium"]}]}
        """.utf8)
        let result = try JSONDecoder().decode(ProviderModelsResponse.self, from: data)
        let model = try #require(result.models.first)
        #expect(model.contextWindow == 128_000)
        #expect(model.supportedReasoningEfforts == ["low", "medium"])
        #expect(model.vision)
    }

    @Test
    func providerAuthRoutesPreserveRemotePrefixesAndEncodeProfiles() throws {
        let base = try #require(URL(string: "https://authority.example/bobe"))
        let url = DaemonClient.endpointURL(
            baseURL: base, pathSegments: ["auth", "profile/name", "login", "submit"]
        )
        #expect(url.absoluteString == "https://authority.example/bobe/auth/profile%2Fname/login/submit")
    }

    @Test
    func cancelBeforeStartupDoesNotStartAnUnattendedLogin() async throws {
        let selection = Self.selection()
        let client = LoginClientFixture()
        let runner = ProviderLoginRunner(
            profile: "provider", selection: selection, client: client, currentSelection: { selection }
        )
        runner.start()
        #expect(await runner.cancel())
        try await Task.sleep(for: .milliseconds(20))
        #expect(await client.startedProfiles.isEmpty)
        #expect(runner.state == .canceled)
    }

    @Test(.timeLimit(.minutes(1)), arguments: ["authorization-response", ""])
    func loginAcceptsInputAndCompletesOnItsPinnedClient(_ value: String) async throws {
        let selection = Self.selection()
        let client = LoginClientFixture()
        let runner = ProviderLoginRunner(
            profile: "openai-oauth", selection: selection, client: client, currentSelection: { selection }
        )
        runner.start()
        await self.waitForInput(runner)
        await runner.submit(value)
        await self.waitForState(runner) { $0 == .completed }
        #expect(runner.state == .completed)
        let submissions = await client.submissions
        #expect(submissions == ["openai-oauth:\(value)"])
        #expect(await client.startedProfiles == ["openai-oauth"])
    }

    @Test(.timeLimit(.minutes(1)))
    func authorityChangeBlocksInputAndCancelsTheOriginalProfile() async throws {
        var selection = Self.selection()
        let client = LoginClientFixture()
        let runner = ProviderLoginRunner(
            profile: "original-provider", selection: selection, client: client, currentSelection: { selection }
        )
        runner.start()
        await self.waitForInput(runner)
        selection = Self.selection()
        await runner.submit("must-not-be-sent")
        #expect(await client.submissions.isEmpty)
        #expect(await runner.cancel())
        #expect(await client.canceledProfiles == ["original-provider"])
        #expect(runner.state == .canceled)
    }

    private func waitForInput(_ runner: ProviderLoginRunner) async {
        await self.waitForState(runner) { state in
            if case .awaitingInput = state { true } else { false }
        }
    }

    private func waitForState(
        _ runner: ProviderLoginRunner,
        where matches: (ProviderLoginRunner.State) -> Bool
    ) async {
        for await state in Observations({ runner.state }) where matches(state) {
            return
        }
    }

    private static func selection() -> EndpointSelection {
        EndpointSelection(generation: 1, profileID: UUID(), authorityID: UUID(), endpoint: .unavailable)
    }
}

private actor LoginClientFixture: ProviderLoginClient {
    private(set) var startedProfiles: [String] = []
    private(set) var canceledProfiles: [String] = []
    private(set) var submissions: [String] = []
    private let events: AsyncStream<ProviderLoginPhase>
    private let continuation: AsyncStream<ProviderLoginPhase>.Continuation

    init() {
        let (events, continuation) = AsyncStream.makeStream(of: ProviderLoginPhase.self)
        self.events = events
        self.continuation = continuation
    }

    func startProviderLogin(profile: String) {
        self.startedProfiles.append(profile)
        self.continuation.yield(.awaitingInput(message: "Paste response", password: true))
    }

    func cancelProviderLogin(profile: String) {
        self.canceledProfiles.append(profile)
        self.continuation.yield(.canceled)
        self.continuation.finish()
    }

    func providerLoginStatus(profile: String) -> ProviderLoginPhase {
        .awaitingInput(message: "Paste response", password: true)
    }

    func submitProviderLogin(profile: String, value: String) {
        self.submissions.append("\(profile):\(value)")
        self.continuation.yield(.completed)
        self.continuation.finish()
    }

    func streamProviderLogin(
        profile: String,
        onPhase: @Sendable @escaping (ProviderLoginPhase) async -> Void
    ) async {
        for await phase in self.events {
            await onPhase(phase)
        }
    }
}
