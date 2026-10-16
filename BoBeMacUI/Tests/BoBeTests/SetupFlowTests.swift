@testable import BoBe
import Foundation
import Testing

@MainActor
private final class Gate {
    private var continuation: CheckedContinuation<Void, any Error>?

    var isWaiting: Bool {
        self.continuation != nil
    }

    func wait() async throws {
        try await withCheckedThrowingContinuation { self.continuation = $0 }
    }

    func fail(_ error: any Error) {
        self.continuation?.resume(throwing: error)
        self.continuation = nil
    }
}

@MainActor
private final class Recorder<Value> {
    var values: [Value] = []
}

@Suite("Setup flow")
struct SetupFlowTests {
    private static func isolatedStore(_ prefix: String) throws -> EndpointStore {
        let defaults = try #require(EphemeralUserDefaults(prefix: prefix))
        let memory = EndpointCredentialMemory()
        return EndpointStore(
            defaults: defaults,
            credentials: EndpointCredentialStore(backing: { .processMemory(memory) })
        )
    }

    private static func loopbackProfile(authority: UUID, epoch: Int) throws -> EndpointProfile {
        EndpointProfile(
            id: UUID(),
            mode: .attachedLocal,
            baseURL: try #require(URL(string: "http://127.0.0.1:18799")),
            runtimeAuthorityId: authority,
            runtimeAuthorityEpoch: epoch,
            endpointId: UUID(),
            friendlyName: "127.0.0.1",
            credentialReference: UUID().uuidString,
            trustMode: .loopback
        )
    }

    private static func hello(authority: UUID, epoch: Int) throws -> AttachResult {
        let json = """
        {"runtime_authority_id":"\(authority.uuidString)","runtime_authority_epoch":\(epoch),\
        "daemon_version":"0.9.1","negotiated_protocol":1,"deployment_mode":"loopback",\
        "initialization_state":"personalization_required","readiness":{"state":"ready","blocking_reasons":[]},\
        "capabilities":{"stt":"ready"},"granted_scopes":[]}
        """
        return AttachResult(hello: try JSONDecoder().decode(HelloResponse.self, from: Data(json.utf8)))
    }

    @Test
    func failedAttachAfterPairingLeavesThePreviousRuntimeForBack() async throws {
        let store = try Self.isolatedStore("SetupFlowTests.failed-attach")
        let previous = try Self.loopbackProfile(authority: UUID(), epoch: 1)
        try store.persist(previous, credential: "previous-credential")
        let selectionBeforePairing = store.selection
        let paired = PairedRuntime(
            profile: try Self.loopbackProfile(authority: UUID(), epoch: 1),
            credential: "paired-credential"
        )

        await #expect(throws: AttachError.self) {
            _ = try await RuntimePairing.commit(paired, store: store) { _, _ in
                throw AttachError.unreachable
            }
        }

        #expect(store.profile == previous)
        #expect(store.credential(for: previous) == "previous-credential")
        #expect(store.credential(for: paired.profile) == nil)
        #expect(store.selection == selectionBeforePairing)
    }

    @Test
    func pairedRuntimeAnsweringWithAnotherIdentityIsNotSaved() async throws {
        let store = try Self.isolatedStore("SetupFlowTests.identity")
        let previous = try Self.loopbackProfile(authority: UUID(), epoch: 1)
        try store.persist(previous, credential: "previous-credential")
        let paired = PairedRuntime(
            profile: try Self.loopbackProfile(authority: UUID(), epoch: 1),
            credential: "paired-credential"
        )
        let impostor = try Self.hello(authority: UUID(), epoch: 1)

        await #expect(throws: RuntimeLocationError.identityMismatch) {
            _ = try await RuntimePairing.commit(paired, store: store) { _, _ in impostor }
        }

        #expect(store.profile == previous)
        #expect(store.credential(for: previous) == "previous-credential")
    }

    @Test
    func verifiedPairingCommitsTheRuntimeAndForgetsTheOldCredential() async throws {
        let store = try Self.isolatedStore("SetupFlowTests.verified")
        let previous = try Self.loopbackProfile(authority: UUID(), epoch: 1)
        try store.persist(previous, credential: "previous-credential")
        let authority = UUID()
        let paired = PairedRuntime(
            profile: try Self.loopbackProfile(authority: authority, epoch: 2),
            credential: "paired-credential"
        )
        let answer = try Self.hello(authority: authority, epoch: 2)

        let state = try await RuntimePairing.commit(paired, store: store) { profile, credential in
            guard profile == paired.profile, credential == paired.credential else {
                throw AttachError.unauthorized
            }
            return answer
        }

        #expect(state == .personalizationRequired)
        #expect(store.profile == paired.profile)
        #expect(store.credential(for: paired.profile) == "paired-credential")
        #expect(store.credential(for: previous) == nil)
    }

    @Test
    @MainActor
    func proactivityStepStaysWhenTheDaemonCannotSaveTheChoice() async {
        let navigation = SetupNavigation(initialStep: .proactivity)
        let requests = Recorder<SettingsUpdateRequest>()

        let advanced = await navigation.commit {
            try await ProactivityLevel.balanced.apply { request in
                requests.values.append(request)
                return SettingsUpdateResponse(message: "kept in memory", restartRequiredFields: [], persistFailed: true)
            }
        }

        #expect(!advanced)
        #expect(navigation.currentStep == .proactivity)
        #expect(navigation.commitError == L10n.tr("setup.proactivity.error.not_saved"))
        #expect(requests.values.first?.captureEnabled == true)
        #expect(requests.values.first?.captureIntervalSeconds == 120)
        #expect(requests.values.first?.checkinEnabled == false)

        let retried = await navigation.commit {
            try await ProactivityLevel.balanced.apply { _ in
                SettingsUpdateResponse(message: "saved", restartRequiredFields: [])
            }
        }

        #expect(retried)
        #expect(navigation.commitError == nil)
    }

    @Test
    @MainActor
    func leavingTheLocalDownloadWaitsForItsCancellationAndStaysWhenItFails() async {
        let navigation = SetupNavigation(initialStep: .welcome)
        navigation.advance(to: .engineChoice)
        navigation.advance(to: .localSetup)
        let gate = Gate()

        let leaving = Task { @MainActor in
            await navigation.goBack(after: {
                try await LocalRuntimeInstall.cancel { try await gate.wait() }
            })
        }
        while !gate.isWaiting {
            await Task.yield()
        }

        #expect(navigation.isCommitting)
        #expect(navigation.currentStep == .localSetup)

        gate.fail(URLError(.timedOut))
        await leaving.value

        #expect(!navigation.isCommitting)
        #expect(navigation.currentStep == .localSetup)
        #expect(navigation.commitError == L10n.tr("setup.local.cancel_failed"))

        await navigation.goBack(after: { try await LocalRuntimeInstall.cancel {} })

        #expect(navigation.currentStep == .engineChoice)
        #expect(navigation.commitError == nil)
    }

    @Test
    func localSetupInstallsTheModelsItActivates() {
        let models = LocalRuntimeModels.setup
        let install = models.installRequest
        let activation = models.activationRequest

        #expect(install.profile == models.profileID)
        #expect(activation.localRuntime == .ollama)
        #expect(activation.chatModel == install.chatModel)
        #expect(activation.goalsModel == install.batchModel)
        #expect(activation.consolidateModel == install.batchModel)
        #expect(activation.smallModel == install.batchModel)
    }
}
