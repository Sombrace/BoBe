import AVFoundation
import Foundation

public enum VoiceReadiness: Equatable {
    case preparing
    case disabledByUser
    case permissionMissing
    case installing
    case modelsMissing
    case failed(String)
    case ready
}

enum VoiceModelPreparation: Equatable {
    case preparing, installing, missing, ready
    case failed(String)

    func resolveReadiness(voiceEnabled: Bool?, permissionMissing: Bool) -> VoiceReadiness {
        if voiceEnabled == false { return .disabledByUser }
        if permissionMissing { return .permissionMissing }
        guard voiceEnabled == true else { return .preparing }
        switch self {
        case .preparing: return .preparing
        case .installing: return .installing
        case .missing: return .modelsMissing
        case .ready: return .ready
        case let .failed(message): return .failed(message)
        }
    }

    static func resolve(
        snapshotAvailable: Bool,
        ttsReady: Bool,
        sttReady: Bool,
        installing: Bool,
        failure: String?
    ) -> Self {
        guard snapshotAvailable else { return .preparing }
        if ttsReady && sttReady { return .ready }
        if let failure { return .failed(failure) }
        return installing ? .installing : .missing
    }
}

enum VoiceSetupPreference {
    private static let deferredKey = "bobe.voice_setup_deferred"

    static var isDeferred: Bool {
        self.isDeferred(in: .standard)
    }

    static func setDeferred(_ deferred: Bool, defaults: UserDefaults = .standard) {
        defaults.set(deferred, forKey: self.deferredKey)
    }

    static func isDeferred(in defaults: UserDefaults) -> Bool {
        defaults.bool(forKey: self.deferredKey)
    }
}

extension VoicePipeline {
    var activeSttLanguage: String {
        Self.sttLanguage(from: SettingsStore.shared.settings)
    }

    var effectiveLanguage: String {
        self.sessionSttLanguage ?? self.activeSttLanguage
    }

    var activeStt: AppleSpeechStt {
        self.recognizer(for: self.effectiveLanguage)
    }

    func recognizer(for language: String) -> AppleSpeechStt {
        if let recognizer = self.recognizers[language] {
            return recognizer
        }
        let pauseMs = Self.eouDelayMs(for: SettingsStore.shared.settings?.voicePauseSensitivity ?? "")
        let recognizer = AppleSpeechStt(language: language, pauseMs: pauseMs)
        self.recognizers[language] = recognizer
        return recognizer
    }

    func daemonRecognizes(_ language: String) -> Bool {
        VoiceTtsBackend.languageKey(language) == "en"
    }

    func refreshSttState() async {
        let language = self.activeSttLanguage
        self.sttStatusLanguage = language
        let ready = self.loadedSttLanguages.contains(language)
            ? true
            : await AppleSpeechStt.assetsInstalled(for: language)
        guard language == self.activeSttLanguage, self.sttDownloadLanguage != language else { return }
        self.sttStatus = ready ? .ready : .notLoaded
    }

    public var readiness: VoiceReadiness {
        VoiceModelInstaller.shared.preparation.resolveReadiness(
            voiceEnabled: SettingsStore.shared.settings?.voiceEnabled,
            permissionMissing: self.permission == .denied || self.permission == .restricted
        )
    }

    func synchronizeDaemonSelection() {
        let selection = EndpointStore.shared.selection
        guard self.daemonStateSelection != selection else { return }
        self.daemonStateSelection = selection
        self.refreshDaemonTask?.cancel()
        self.refreshDaemonTask = nil
        self.installSnapshot = nil
        self.daemonSttReady = false
    }

    public func refreshDaemonState() async {
        self.synchronizeDaemonSelection()
        if let existing = self.refreshDaemonTask {
            await existing.value
            return
        }
        let selection = self.daemonStateSelection
        let client = DaemonClient(endpoint: selection.endpoint)
        let task = Task {
            async let install: VoiceInstallSnapshot? = try? await client.voiceInstallStatus()
            async let attach: AttachResult? =
                try? await EndpointAttachService.shared.attachConfiguredEndpoint(selection: selection)
            async let settings: Void = SettingsStore.shared.refresh()
            let (installRes, attachRes, _) = await (install, attach, settings)
            guard EndpointStore.shared.selection == selection else { return }
            if let attachRes {
                self.daemonSttReady = attachRes.hello.capabilities.stt == .ready
            }
            if let installRes {
                self.installSnapshot = installRes
                self.installSnapshotRevision &+= 1
            }
        }
        self.refreshDaemonTask = task
        await task.value
        guard EndpointStore.shared.selection == selection else {
            if self.refreshDaemonTask != nil, self.daemonStateSelection == selection {
                self.refreshDaemonTask = nil
                await self.refreshDaemonState()
            }
            return
        }
        self.refreshDaemonTask = nil
        self.reconcileVoiceSettings()
        VoiceModelInstaller.shared.monitorIfInstalling()
    }

    func reconcileVoiceSettings() {
        if self.sttStatusLanguage != self.activeSttLanguage {
            self.sttStatusLanguage = self.activeSttLanguage
            Task { await self.refreshSttState() }
        }
        guard let settings = SettingsStore.shared.settings else { return }
        let delayMs = Self.eouDelayMs(for: settings.voicePauseSensitivity)
        let recognizers = Array(self.recognizers.values)
        Task {
            for recognizer in recognizers {
                await recognizer.setPauseMs(delayMs)
            }
        }
        if !settings.voiceEnabled, self.state != .idle {
            self.disconnect()
        }
    }

    static func eouDelayMs(for sensitivity: String) -> Int {
        switch sensitivity.lowercased() {
        case "tight": PauseSensitivityMs.tight
        case "patient": PauseSensitivityMs.patient
        default: PauseSensitivityMs.balanced
        }
    }

    public func updatePermission(_ status: AVAuthorizationStatus) {
        self.permission = status
    }
}
