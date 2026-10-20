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
    func failedSelectionRetriesTheDesiredTheme() async {
        let client = SettingsClientStub(
            settings: SettingsClientStub.document(theme: .warmLedge),
            themeOutcomes: [.failure, .success(delivered: true)]
        )
        let store = SettingsStore(client: client)
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
    func stalePollCannotOverwriteCompletedSelection() async {
        let initial = SettingsClientStub.document(theme: .warmLedge)
        let client = SettingsClientStub(
            settings: initial,
            themeOutcomes: [.success(delivered: true)]
        )
        let store = SettingsStore(client: client)
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
    func authoritySwitchCancelsQueuedSettingsAndReloads() async {
        var selection = SettingsClientStub.selection(generation: 1)
        let client = SettingsClientStub(settings: SettingsClientStub.document(theme: .warmLedge))
        let store = SettingsStore(client: client, currentSelection: { selection })
        await store.loadIfNeeded()
        store.update { $0.captureEnabled = true }
        selection = SettingsClientStub.selection(generation: 2)
        store.synchronizeEndpoint()
        #expect(store.settings == nil)
        await store.flushPendingSave()
        #expect(await client.sentRequests().isEmpty)
        await store.loadIfNeeded()
        #expect(store.settings?.captureEnabled == false)
    }

    @Test
    func previousAuthorityLoadCannotReplaceNewAuthority() async {
        var selection = SettingsClientStub.selection(generation: 1)
        let client = SettingsClientStub(settings: SettingsClientStub.document(theme: .warmLedge))
        let store = SettingsStore(client: client, currentSelection: { selection })
        await client.suspendNextSettingsFetch()
        let oldLoad = Task { await store.loadIfNeeded() }
        while !(await client.hasSuspendedFetch()) { await Task.yield() }
        selection = SettingsClientStub.selection(generation: 2)
        await store.loadIfNeeded()
        await client.resumeSettingsFetch(with: SettingsClientStub.document(theme: .solarCrown))
        await oldLoad.value
        #expect(store.settings?.bodyTheme == .warmLedge)
    }

    @Test
    func previousAuthoritySaveCannotPublishCompletion() async {
        var selection = SettingsClientStub.selection(generation: 1)
        let client = SettingsClientStub(
            settings: SettingsClientStub.document(theme: .warmLedge), themeOutcomes: [.success(delivered: true)]
        )
        let store = SettingsStore(client: client, currentSelection: { selection })
        await store.loadIfNeeded()
        await client.suspendNextUpdate()
        let save = Task { await store.setBodyTheme(.solarCrown) }
        while !(await client.hasSuspendedUpdate()) { await Task.yield() }
        selection = SettingsClientStub.selection(generation: 2)
        store.synchronizeEndpoint()
        await store.loadIfNeeded()
        await client.resumeUpdate()
        await save.value
        #expect(store.settings?.bodyTheme == .warmLedge)
        #expect(store.bodyThemeSaveState == .idle)
        #expect(store.restartFields.isEmpty)
    }

    @Test
    func daemonRestartRequirementsAreAuthoritative() async {
        let client = SettingsClientStub(
            settings: SettingsClientStub.document(theme: .warmLedge),
            themeOutcomes: [.success(delivered: true)],
            restartRequiredFields: ["example_static_field"]
        )
        let store = SettingsStore(client: client)
        await store.loadIfNeeded()

        await store.setBodyTheme(.solarCrown)

        #expect(store.restartFields == Set(["example_static_field"]))
    }

    @Test
    func refreshAdoptsDaemonSettingsButLeavesBodyThemeFlow() async {
        let client = SettingsClientStub(settings: SettingsClientStub.document(theme: .warmLedge))
        let store = SettingsStore(client: client)
        await store.loadIfNeeded()
        var changed = SettingsClientStub.document(theme: .solarCrown)
        changed.bodyConnected = false
        changed.voiceEnabled = false
        changed.voiceSttLanguage = "de"
        await client.replaceSettings(changed)

        await store.refresh()

        #expect(store.settings?.voiceEnabled == false)
        #expect(store.settings?.voiceSttLanguage == "de")
        #expect(store.settings?.bodyTheme == .warmLedge)
        #expect(store.settings?.bodyConnected == true)
    }

    @Test
    func refreshKeepsUnsavedLocalEdit() async {
        let client = SettingsClientStub(settings: SettingsClientStub.document(theme: .warmLedge))
        let store = SettingsStore(client: client)
        await store.loadIfNeeded()
        store.update { $0.voiceEnabled = false }

        await store.refresh()

        #expect(store.settings?.voiceEnabled == false)
        await store.flushPendingSave()
    }

    @Test
    func editDuringRefreshWins() async {
        let client = SettingsClientStub(settings: SettingsClientStub.document(theme: .warmLedge))
        let store = SettingsStore(client: client)
        await store.loadIfNeeded()
        await client.suspendNextSettingsFetch()
        let refresh = Task { await store.refresh() }
        while !(await client.hasSuspendedFetch()) { await Task.yield() }

        store.update { $0.voiceSpeed = 1.5 }
        await client.resumeSettingsFetch(with: SettingsClientStub.document(theme: .warmLedge))
        await refresh.value

        #expect(store.settings?.voiceSpeed == 1.5)
        await store.flushPendingSave()
    }
}
