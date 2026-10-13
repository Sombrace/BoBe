@testable import BoBe
import Foundation
import Testing

@Suite("Physical body theme lifecycle")
struct BodyThemeLifecycleTests {
    @Test
    func onlyUndeliveredOrFailedSelectionsAllowSameThemeRetry() {
        #expect(BodyThemeSaveState.savedOffline.allowsRetry)
        #expect(BodyThemeSaveState.failed("delivery failed").allowsRetry)
        #expect(!BodyThemeSaveState.idle.allowsRetry)
        #expect(!BodyThemeSaveState.applying.allowsRetry)
        #expect(!BodyThemeSaveState.applied.allowsRetry)
    }

    @Test
    func refreshTracksConnectivityWithoutErasingFailure() {
        #expect(BodyThemeSaveState.idle.refreshed(bodyConnected: true) == .idle)
        #expect(BodyThemeSaveState.savedOffline.refreshed(bodyConnected: true) == .applied)
        #expect(BodyThemeSaveState.applied.refreshed(bodyConnected: false) == .savedOffline)
        #expect(
            BodyThemeSaveState.failed("delivery failed").refreshed(bodyConnected: true)
                == .failed("delivery failed")
        )
    }

    @Test
    @MainActor
    func failedSelectionRetriesTheDesiredTheme() async {
        let client = BodyThemeClientStub(
            settings: makeSettings(theme: .warmLedge),
            updateOutcomes: [.failure, .success(delivered: true)]
        )
        let store = SettingsStore(bodyThemeClient: client)
        await store.loadIfNeeded()

        await store.setBodyTheme(.solarCrown)
        #expect(store.settings?.bodyTheme == .solarCrown)
        if case .failed = store.bodyThemeSaveState {
        } else {
            Issue.record("A failed delivery should remain retryable")
        }

        await store.setBodyTheme(.solarCrown)

        #expect(store.settings?.bodyTheme == .solarCrown)
        #expect(store.bodyThemeSaveState == .applied)
        #expect(await client.requestedThemes() == [.solarCrown, .solarCrown])
    }

    @Test
    @MainActor
    func stalePollCannotOverwriteCompletedSelection() async {
        let initial = makeSettings(theme: .warmLedge)
        let client = BodyThemeClientStub(
            settings: initial,
            updateOutcomes: [.success(delivered: true)]
        )
        let store = SettingsStore(bodyThemeClient: client)
        await store.loadIfNeeded()
        await client.suspendNextSettingsFetch()

        let refresh = Task { await store.refreshBodyThemeStatus() }
        while !(await client.hasSuspendedFetch()) {
            await Task.yield()
        }

        await store.setBodyTheme(.solarCrown)
        await client.resumeSettingsFetch(with: initial)
        await refresh.value

        #expect(store.settings?.bodyTheme == .solarCrown)
        #expect(store.bodyThemeSaveState == .applied)
    }

    @Test
    @MainActor
    func authoritySwitchCancelsQueuedSettingsAndReloads() async {
        var selection = testSelection(generation: 1)
        let client = BodyThemeClientStub(settings: makeSettings(theme: .warmLedge), updateOutcomes: [])
        let store = SettingsStore(bodyThemeClient: client, currentSelection: { selection })
        await store.loadIfNeeded()
        store.update { $0.captureEnabled = true }
        selection = testSelection(generation: 2)
        store.synchronizeEndpoint()
        #expect(store.settings == nil)
        await store.flushPendingSave()
        #expect(await client.requestedThemes().isEmpty)
        await store.loadIfNeeded()
        #expect(store.settings?.captureEnabled == false)
    }

    @Test
    @MainActor
    func previousAuthorityLoadCannotReplaceNewAuthority() async {
        var selection = testSelection(generation: 1)
        let client = BodyThemeClientStub(settings: makeSettings(theme: .warmLedge), updateOutcomes: [])
        let store = SettingsStore(bodyThemeClient: client, currentSelection: { selection })
        await client.suspendNextSettingsFetch()
        let oldLoad = Task { await store.loadIfNeeded() }
        while !(await client.hasSuspendedFetch()) { await Task.yield() }
        selection = testSelection(generation: 2)
        await store.loadIfNeeded()
        await client.resumeSettingsFetch(with: makeSettings(theme: .solarCrown))
        await oldLoad.value
        #expect(store.settings?.bodyTheme == .warmLedge)
    }

    @Test
    @MainActor
    func previousAuthoritySaveCannotPublishCompletion() async {
        var selection = testSelection(generation: 1)
        let client = BodyThemeClientStub(
            settings: makeSettings(theme: .warmLedge), updateOutcomes: [.success(delivered: true)]
        )
        let store = SettingsStore(bodyThemeClient: client, currentSelection: { selection })
        await store.loadIfNeeded()
        await client.suspendNextUpdate()
        let save = Task { await store.setBodyTheme(.solarCrown) }
        while !(await client.hasSuspendedUpdate()) { await Task.yield() }
        selection = testSelection(generation: 2)
        store.synchronizeEndpoint()
        await store.loadIfNeeded()
        await client.resumeUpdate()
        await save.value
        #expect(store.settings?.bodyTheme == .warmLedge)
        #expect(store.bodyThemeSaveState == .idle)
        #expect(store.restartFields.isEmpty)
    }

    @Test
    @MainActor
    func daemonRestartRequirementsAreAuthoritative() async {
        let client = BodyThemeClientStub(
            settings: makeSettings(theme: .warmLedge),
            updateOutcomes: [.success(delivered: true)],
            restartRequiredFields: ["example_static_field"]
        )
        let store = SettingsStore(bodyThemeClient: client)
        await store.loadIfNeeded()

        await store.setBodyTheme(.solarCrown)

        #expect(store.restartFields == Set(["example_static_field"]))
    }
}

private actor BodyThemeClientStub: BodyThemeSettingsClient {
    enum UpdateOutcome {
        case failure
        case success(delivered: Bool)
    }

    private var settings: DaemonSettings
    private var updateOutcomes: [UpdateOutcome]
    private let restartRequiredFields: [String]
    private var themes: [PhysicalBodyTheme] = []
    private var shouldSuspendNextFetch = false
    private var fetchContinuation: CheckedContinuation<DaemonSettings, Never>?
    private var suspendUpdate = false
    private var updateContinuation: CheckedContinuation<Void, Never>?

    func suspendNextUpdate() { self.suspendUpdate = true }
    func hasSuspendedUpdate() -> Bool { self.updateContinuation != nil }
    func resumeUpdate() {
        self.updateContinuation?.resume()
        self.updateContinuation = nil
    }

    init(
        settings: DaemonSettings,
        updateOutcomes: [UpdateOutcome],
        restartRequiredFields: [String] = []
    ) {
        self.settings = settings
        self.updateOutcomes = updateOutcomes
        self.restartRequiredFields = restartRequiredFields
    }

    func getSettings() async -> DaemonSettings {
        if self.shouldSuspendNextFetch {
            self.shouldSuspendNextFetch = false
            return await withCheckedContinuation { continuation in
                self.fetchContinuation = continuation
            }
        }
        return self.settings
    }

    func updateSettings(_ request: SettingsUpdateRequest) async throws -> SettingsUpdateResponse {
        if self.suspendUpdate {
            self.suspendUpdate = false
            await withCheckedContinuation { self.updateContinuation = $0 }
        }
        let theme = try #require(request.bodyTheme)
        self.themes.append(theme)
        let outcome = self.updateOutcomes.removeFirst()
        switch outcome {
        case .failure:
            throw BodyThemeClientStubError.updateFailed
        case let .success(delivered):
            self.settings.bodyTheme = theme
            self.settings.bodyConnected = delivered
            return SettingsUpdateResponse(
                message: "updated",
                restartRequiredFields: self.restartRequiredFields,
                persistFailed: nil,
                bodyThemeDelivered: delivered
            )
        }
    }

    func suspendNextSettingsFetch() {
        self.shouldSuspendNextFetch = true
    }

    func hasSuspendedFetch() -> Bool {
        self.fetchContinuation != nil
    }

    func resumeSettingsFetch(with settings: DaemonSettings) {
        self.fetchContinuation?.resume(returning: settings)
        self.fetchContinuation = nil
    }

    func requestedThemes() -> [PhysicalBodyTheme] {
        self.themes
    }
}

private enum BodyThemeClientStubError: Error {
    case updateFailed
}

private func testSelection(generation: UInt64) -> EndpointSelection {
    EndpointSelection(generation: generation, profileID: UUID(), authorityID: UUID(), endpoint: .unavailable)
}

private func makeSettings(theme: PhysicalBodyTheme) -> DaemonSettings {
    DaemonSettings(
        bodyTheme: theme,
        bodyConnected: true,
        captureEnabled: false,
        captureIntervalSeconds: 30,
        checkinEnabled: false,
        checkinTimes: [],
        checkinJitterMinutes: 0,
        conversationInactivityTimeoutSeconds: 300,
        conversationAutoCloseMinutes: 30,
        goalCheckIntervalSeconds: 60,
        mcpEnabled: false,
        voiceEnabled: true,
        voicePersona: "default",
        voiceSpeed: 1,
        voiceSttLanguage: "en",
        voicePauseSensitivity: "balanced",
        voiceShowPartialCaption: true
    )
}
