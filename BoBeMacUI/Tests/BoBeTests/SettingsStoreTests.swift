@testable import BoBe
import Foundation
import Testing

@Suite("Settings field-level saves")
struct SettingsStoreTests {
    @Test
    func externalChangeSurvivesAnUnrelatedEdit() async throws {
        let client = SettingsClientStub(settings: SettingsClientStub.document())
        let store = SettingsStore(client: client)
        await store.loadIfNeeded()
        var external = SettingsClientStub.document()
        external.captureEnabled = true
        await client.replaceSettings(external)

        store.update { $0.voiceSpeed = 1.5 }
        await store.flushPendingSave()

        #expect(await client.currentSettings().captureEnabled)
        #expect(try Self.sentFields(await client.sentRequests()) == [["voice_speed"]])
        await store.loadIfNeeded(force: true)
        #expect(store.settings?.captureEnabled == true)
        #expect(store.settings?.voiceSpeed == 1.5)
    }

    @Test
    func menuCaptureChangeSurvivesALaterSettingsEdit() async throws {
        let selection = SettingsClientStub.selection(generation: 1)
        var document = SettingsClientStub.document()
        document.captureEnabled = true
        let client = SettingsClientStub(settings: document)
        let settings = SettingsStore(client: client, currentSelection: { selection })
        let store = BobeStore(currentSelection: { selection }, settingsStore: settings)
        await settings.loadIfNeeded()
        #expect(store.isCapturing)

        #expect(await store.setCapture(false) == false)
        #expect(!store.isCapturing)
        settings.update { $0.voiceSpeed = 1.25 }
        await settings.flushPendingSave()

        let daemon = await client.currentSettings()
        #expect(!daemon.captureEnabled)
        #expect(daemon.voiceSpeed == 1.25)
        #expect(try Self.sentFields(await client.sentRequests()) == [["capture_enabled"], ["voice_speed"]])
        #expect(!store.isCapturing)
    }

    @Test
    func forcedLoadRightAfterAnEditKeepsTheEditedValue() async {
        let client = SettingsClientStub(settings: SettingsClientStub.document())
        let store = SettingsStore(client: client)
        await store.loadIfNeeded()

        store.update { $0.voiceSpeed = 1.5 }
        await store.loadIfNeeded(force: true)

        #expect(store.settings?.voiceSpeed == 1.5)
        #expect(await client.currentSettings().voiceSpeed == 1.5)
    }

    @Test
    func forcedLoadDuringAnUnsentEditKeepsTheEdit() async {
        let client = SettingsClientStub(settings: SettingsClientStub.document())
        let store = SettingsStore(client: client)
        await store.loadIfNeeded()
        await client.suspendNextSettingsFetch()
        let load = Task { await store.loadIfNeeded(force: true) }
        while !(await client.hasSuspendedFetch()) { await Task.yield() }

        store.update { $0.voiceSpeed = 1.5 }
        await client.resumeSettingsFetch(with: SettingsClientStub.document())
        await load.value

        #expect(store.settings?.voiceSpeed == 1.5)
        await store.flushPendingSave()
        #expect(await client.currentSettings().voiceSpeed == 1.5)
    }

    @Test
    func queuedEditsMergeIntoOneSaveOfOnlyTheChangedFields() async throws {
        let client = SettingsClientStub(settings: SettingsClientStub.document())
        let store = SettingsStore(client: client)
        await store.loadIfNeeded()

        store.update { $0.voiceSpeed = 1.25 }
        store.update { $0.checkinTimes = ["09:00"] }
        store.update { $0.voiceSpeed = 1.5 }
        store.update { $0.voiceEnabled = true }
        await store.flushPendingSave()

        #expect(try Self.sentFields(await client.sentRequests()) == [["checkin_times", "voice_speed"]])
        let daemon = await client.currentSettings()
        #expect(daemon.voiceSpeed == 1.5)
        #expect(daemon.checkinTimes == ["09:00"])
    }

    @Test
    func failedSaveShowsTheDaemonValueAgain() async {
        let client = SettingsClientStub(settings: SettingsClientStub.document(), failingFieldUpdates: 1)
        let store = SettingsStore(client: client)
        await store.loadIfNeeded()

        let saved = await store.setCaptureEnabled(true)

        #expect(!saved)
        #expect(store.settings?.captureEnabled == false)
        #expect(store.error != nil)
        #expect(await client.currentSettings().captureEnabled == false)
    }

    private static func sentFields(_ requests: [SettingsUpdateRequest]) throws -> [[String]] {
        try requests.map { request in
            let object = try JSONSerialization.jsonObject(with: JSONEncoder().encode(request))
            return ((object as? [String: Any]) ?? [:]).keys.sorted()
        }
    }
}
