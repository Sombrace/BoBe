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

@MainActor
extension VoicePipeline {
    var activeSttLanguage: String {
        let language = SettingsStore.shared.settings?.voiceSttLanguage ?? ""
        return language.isEmpty ? "en" : language
    }

    var effectiveLanguage: String {
        self.sessionSttLanguage ?? self.activeSttLanguage
    }

    var activeStt: any VoiceSttEngine {
        self.sttEngine(
            engineId: self.effectiveSttEngineId,
            language: self.effectiveLanguage
        )
    }

    func preferredSttEngineId(for language: String) -> String {
        switch VoiceSttPreferenceStore.shared.preference {
        case .appleSpeech:
            VoiceSttEngineId.appleSpeechAnalyzer
        case .automatic, .fluidAudio, .daemon:
            self.fluidAudioEngineId(for: language)
        }
    }

    func fluidAudioEngineId(for language: String) -> String {
        language == "en"
            ? VoiceSttEngineId.fluidAudioParakeet
            : VoiceSttEngineId.fluidAudioNemotron
    }

    func sttEngine(engineId: String, language: String) -> any VoiceSttEngine {
        switch engineId {
        case VoiceSttEngineId.appleSpeechAnalyzer:
            if let engine = self.appleSttByLanguage[language] {
                return engine
            }
            let engine = AppleSpeechStt(language: language)
            self.appleSttByLanguage[language] = engine
            return engine
        case VoiceSttEngineId.fluidAudioParakeet:
            return self.parakeetStt
        default:
            return self.nemotronStt
        }
    }

    static func sttReadinessKey(engineId: String, language: String) -> String {
        "\(engineId):\(language)"
    }

    func refreshSttPreferenceState() {
        self.sttStatusLanguage = self.activeSttLanguage
        if VoiceSttPreferenceStore.shared.preference == .daemon {
            self.sttStatus = self.daemonSttReady ? .ready : .notLoaded
            return
        }
        let engineId = self.preferredSttEngineId(for: self.activeSttLanguage)
        let key = Self.sttReadinessKey(engineId: engineId, language: self.activeSttLanguage)
        if self.loadedSttKeys.contains(key) {
            self.sttStatus = .ready
        } else if engineId != VoiceSttEngineId.appleSpeechAnalyzer,
                  self.modelIsInstalled(for: self.activeSttLanguage) {
            self.sttStatus = .ready
        } else {
            self.sttStatus = .notLoaded
        }
    }

    var activeModelIsInstalled: Bool {
        let engineId = self.preferredSttEngineId(for: self.effectiveLanguage)
        if engineId == VoiceSttEngineId.appleSpeechAnalyzer {
            return self.loadedSttKeys.contains(
                Self.sttReadinessKey(engineId: engineId, language: self.effectiveLanguage)
            )
        }
        return self.modelIsInstalled(for: self.effectiveLanguage)
    }

    func modelIsInstalled(for language: String) -> Bool {
        switch language {
        case "en": FluidAudioModelPresence.isInstalled()
        default: FluidAudioNemotronModelPresence.isInstalled()
        }
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
        let task = Task { @MainActor in
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
            self.refreshSttPreferenceState()
        }
        guard let settings = SettingsStore.shared.settings else { return }
        let delayMs = Self.eouDelayMs(for: settings.voicePauseSensitivity)
        let nemotron = self.nemotronStt
        let parakeet = self.parakeetStt
        let appleEngines = Array(self.appleSttByLanguage.values)
        Task {
            await nemotron.setEouDelayMs(delayMs)
            await parakeet.setEouDebounceMs(delayMs)
            for engine in appleEngines {
                await engine.setEouDebounceMs(delayMs)
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
