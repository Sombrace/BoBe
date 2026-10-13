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

    var modelPreparation: VoiceModelPreparation {
        let backend = VoiceTtsPreference.shared.backend
        let preference = VoiceSttPreferenceStore.shared.preference
        let ttsReady = backend == .serverKokoro
            ? self.installSnapshot?.installed.tts == true
            : FluidAudioSupertonicModelPresence.isInstalled()
        let sttReady = preference.speechRecognitionReady(
            endpointReady: preference.usesEndpointEngine && self.sttStatus == .ready,
            daemonReady: self.daemonSttReady
        )
        let failure: String?
        if case let .failed(message) = self.sttStatus, !sttReady {
            failure = message
        } else if backend == .serverKokoro,
                  self.installSnapshot?.status == .failed, !ttsReady {
            failure = self.installSnapshot?.error ?? L10n.tr("setup.voice.poll_failure")
        } else {
            failure = nil
        }
        return .resolve(
            snapshotAvailable: self.installSnapshot != nil,
            ttsReady: ttsReady,
            sttReady: sttReady,
            installing: (backend == .serverKokoro && self.installSnapshot?.isRunning == true)
                || (preference.usesEndpointEngine && self.sttStatus == .downloading),
            failure: failure
        )
    }

    public var readiness: VoiceReadiness {
        if self.voiceEnabled == false { return .disabledByUser }
        if self.permission == .denied || self.permission == .restricted { return .permissionMissing }
        return self.modelPreparation.resolveReadiness(
            voiceEnabled: self.voiceEnabled,
            permissionMissing: self.permission == .denied || self.permission == .restricted
        )
    }

    func synchronizeDaemonSelection() {
        let selection = EndpointStore.shared.selection
        guard self.daemonStateSelection != selection else { return }
        self.daemonStateSelection = selection
        self.refreshDaemonTask?.cancel()
        self.refreshDaemonTask = nil
        self.installPollTask?.cancel()
        self.installPollTask = nil
        self.installSnapshot = nil
        self.voiceEnabled = nil
        self.daemonSttReady = false
    }

    func startDaemonTtsInstall() async throws {
        self.synchronizeDaemonSelection()
        let selection = self.daemonStateSelection
        let client = DaemonClient(endpoint: selection.endpoint)
        try await client.startVoiceInstall(kind: VoiceWire.modelKindTts)
        guard EndpointStore.shared.selection == selection else { return }
        if let refresh = self.refreshDaemonTask { await refresh.value }
        guard EndpointStore.shared.selection == selection else { return }
        self.monitorDaemonInstall()
    }

    func monitorDaemonInstall() {
        guard self.installPollTask == nil else { return }
        let selection = self.daemonStateSelection
        self.installPollTask = Task { @MainActor [weak self] in
            guard let self else { return }
            defer {
                if self.daemonStateSelection == selection { self.installPollTask = nil }
            }
            while !Task.isCancelled, EndpointStore.shared.selection == selection {
                let revision = self.installSnapshotRevision
                await self.refreshDaemonState()
                guard !Task.isCancelled, EndpointStore.shared.selection == selection else { return }
                if revision != self.installSnapshotRevision, self.installSnapshot?.isTerminal == true { return }
                do { try await Task.sleep(for: .seconds(1)) } catch { return }
            }
        }
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
            async let settings: DaemonSettings? = try? await client.getSettings()
            async let attach: AttachResult? =
                try? await EndpointAttachService.shared.attachConfiguredEndpoint(selection: selection)
            let (installRes, settingsRes, attachRes) = await (install, settings, attach)
            guard EndpointStore.shared.selection == selection else { return }
            if let attachRes {
                self.daemonSttReady = attachRes.hello.capabilities.stt == .ready
            }
            if let installRes {
                self.installSnapshot = installRes
                self.installSnapshotRevision &+= 1
            }
            if let settingsRes { self.applySettings(settingsRes) }
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
        if self.installSnapshot?.isRunning == true { self.monitorDaemonInstall() }
    }

    func applySettings(_ settings: DaemonSettings) {
        self.voiceEnabled = settings.voiceEnabled
        self.showPartialCaption = settings.voiceShowPartialCaption
        self.voicePersona = settings.voicePersona
        self.voiceSpeed = settings.voiceSpeed
        let language = settings.voiceSttLanguage
        if !language.isEmpty, language != self.activeSttLanguage {
            self.activeSttLanguage = language
            let engineId = self.preferredSttEngineId(for: language)
            let key = Self.sttReadinessKey(engineId: engineId, language: language)
            self.sttStatus = self.loadedSttKeys.contains(key)
                || (engineId != VoiceSttEngineId.appleSpeechAnalyzer
                    && self.modelIsInstalled(for: language)) ? .ready : .notLoaded
        }
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
