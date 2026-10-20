import Foundation
import Observation

enum VoiceModel: Hashable, Sendable {
    case pocketTts
    case supertonicTts
    case englishStt
    case multilingualStt

    var installPath: String {
        switch self {
        case .pocketTts:
            DaemonConfig.managedDataDirectory
                .appendingPathComponent("models", isDirectory: true)
                .appendingPathComponent(VoiceWire.pocketTtsModelDir, isDirectory: true)
                .path
        case .supertonicTts:
            "~/.cache/fluidaudio/Models/supertonic-3/"
        case .englishStt:
            "~/Library/Application Support/FluidAudio/Models/parakeet-eou-streaming/"
        case .multilingualStt:
            "~/Library/Application Support/FluidAudio/Models/nemotron-multilingual/"
        }
    }
}

enum VoiceModelStatus: Equatable, Sendable {
    case unknown
    case missing
    case downloading(percent: Int?, bytesDownloaded: UInt64 = 0, bytesTotal: UInt64? = nil)
    case installed
    case failed(String)
}

enum VoiceInstallOutcome: Equatable, Sendable {
    case installing
    case ready
    case stopped
    case failed(String)

    static func resolve(
        installing: Bool,
        snapshotRunning: Bool?,
        preparation: VoiceModelPreparation,
        sttStatus: VoicePipeline.SttStatus,
        snapshotRevision: UInt64 = 0,
        staleThrough: UInt64? = nil
    ) -> Self {
        if installing { return .installing }
        guard let snapshotRunning else { return .failed(L10n.tr("setup.voice.daemon_unreachable")) }
        if let staleThrough, snapshotRevision <= staleThrough { return .installing }
        if preparation == .ready { return .ready }
        if case let .failed(message) = sttStatus { return .failed(message) }
        if snapshotRunning || sttStatus == .downloading { return .installing }
        if case let .failed(message) = preparation { return .failed(message) }
        return .stopped
    }
}

@Observable
final class VoiceModelInstaller {
    static let shared = VoiceModelInstaller()

    private(set) var isInstalling = false
    private(set) var isPreparingTts = false
    private(set) var supertonicInstalled = FluidAudioSupertonicModelPresence.isInstalled()
    private var staleSnapshotRevision: UInt64?

    @ObservationIgnored private var installGeneration: UInt64 = 0
    @ObservationIgnored private var monitorTask: Task<Void, Never>?
    @ObservationIgnored private var monitorSelection: EndpointSelection?

    private var pipeline: VoicePipeline {
        VoicePipeline.shared
    }

    private init() {}

    var selectedTtsReady: Bool {
        switch VoiceTtsPreference.shared.backend {
        case .serverPocket: self.pipeline.installSnapshot?.ttsReady == true
        case .clientSupertonic: self.supertonicInstalled
        }
    }

    var preparation: VoiceModelPreparation {
        let pipeline = self.pipeline
        let backend = VoiceTtsPreference.shared.backend
        let preference = VoiceSttPreferenceStore.shared.preference
        let snapshot = pipeline.installSnapshot
        guard backend.speaks(pipeline.effectiveLanguage) else {
            return .failed(L10n.tr(
                "settings.voice.language_unsupported",
                backend.shortName,
                VoiceLanguages.displayName(for: pipeline.effectiveLanguage)
            ))
        }
        let ttsReady = self.selectedTtsReady
        let sttReady = preference.speechRecognitionReady(
            endpointReady: preference.usesEndpointEngine && pipeline.sttStatus == .ready,
            daemonReady: pipeline.daemonSttReady
        )
        let failure: String?
        if case let .failed(message) = pipeline.sttStatus, !sttReady {
            failure = message
        } else if backend == .serverPocket, snapshot?.status == .failed, !ttsReady {
            failure = snapshot?.error ?? L10n.tr("setup.voice.poll_failure")
        } else {
            failure = nil
        }
        return .resolve(
            snapshotAvailable: snapshot != nil,
            ttsReady: ttsReady,
            sttReady: sttReady,
            installing: (backend == .serverPocket && snapshot?.isRunning == true)
                || (preference.usesEndpointEngine && pipeline.sttStatus == .downloading),
            failure: failure
        )
    }

    var outcome: VoiceInstallOutcome {
        .resolve(
            installing: self.isInstalling,
            snapshotRunning: self.pipeline.installSnapshot?.isRunning,
            preparation: self.preparation,
            sttStatus: self.pipeline.sttStatus,
            snapshotRevision: self.pipeline.installSnapshotRevision,
            staleThrough: self.staleSnapshotRevision
        )
    }

    private var awaitsDaemonStatus: Bool {
        guard let stale = self.staleSnapshotRevision else { return false }
        return self.pipeline.installSnapshotRevision <= stale
    }

    var selectedSttStatus: VoiceModelStatus {
        switch self.pipeline.sttStatus {
        case .notLoaded: .missing
        case .downloading: .downloading(percent: self.pipeline.sttLoadProgress?.percent)
        case .ready: .installed
        case let .failed(message): .failed(message)
        }
    }

    func status(of model: VoiceModel) -> VoiceModelStatus {
        switch model {
        case .pocketTts: self.pocketStatus
        case .supertonicTts: self.supertonicInstalled ? .installed : .missing
        case .englishStt: self.sttStatus(FluidAudioModelPresence.self, english: true)
        case .multilingualStt: self.sttStatus(FluidAudioNemotronModelPresence.self, english: false)
        }
    }

    func refresh() async {
        await self.pipeline.refreshDaemonState()
        if self.pipeline.sttStatus == .notLoaded { self.pipeline.bootstrapSttPresence() }
        self.supertonicInstalled = FluidAudioSupertonicModelPresence.isInstalled()
    }

    func installSelected() async throws {
        VoiceSetupPreference.setDeferred(false)
        self.installGeneration &+= 1
        let generation = self.installGeneration
        self.isInstalling = true
        defer { if generation == self.installGeneration { self.isInstalling = false } }
        async let stt: Void = self.pipeline.ensureSttLoaded()
        try await self.prepareSelectedTts()
        await stt
    }

    func resume() {
        Task { await self.pipeline.ensureSttLoaded() }
    }

    func install(_ model: VoiceModel) async throws {
        VoiceSetupPreference.setDeferred(false)
        do {
            switch model {
            case .pocketTts:
                try await self.startDaemonTts()
            case .supertonicTts:
                try await self.prepareSupertonic()
            case .englishStt:
                await self.pipeline.ensureSttLoaded(for: "en")
            case .multilingualStt:
                let language = self.pipeline.activeSttLanguage
                await self.pipeline.ensureSttLoaded(for: language == "en" ? "zh" : language)
            }
        } catch {
            await self.refresh()
            throw error
        }
        await self.refresh()
    }

    func uninstall(_ model: VoiceModel) async {
        let engineId: String
        switch model {
        case .englishStt: engineId = VoiceSttEngineId.fluidAudioParakeet
        case .multilingualStt: engineId = VoiceSttEngineId.fluidAudioNemotron
        case .pocketTts, .supertonicTts: return
        }
        await self.pipeline.unloadSttEngine(engineId)
        let path = NSString(string: model.installPath).expandingTildeInPath
        if FileManager.default.fileExists(atPath: path) {
            try? FileManager.default.removeItem(at: URL(fileURLWithPath: path, isDirectory: true))
        }
        await self.refresh()
    }

    func cancel() async throws {
        self.installGeneration &+= 1
        self.isInstalling = false
        self.staleSnapshotRevision = nil
        let pipeline = self.pipeline
        let selection = pipeline.daemonStateSelection
        async let stt: Void = pipeline.cancelSttLoading()
        let daemon: Result<Void, any Error>
        do {
            try await DaemonClient(endpoint: selection.endpoint).cancelVoiceInstall()
            daemon = .success(())
        } catch {
            daemon = .failure(error)
        }
        await stt
        try daemon.get()
        try await self.awaitDaemonInstallStopped(selection: selection)
    }

    private func awaitDaemonInstallStopped(selection: EndpointSelection) async throws {
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: .seconds(15))
        while EndpointStore.shared.selection == selection {
            await self.pipeline.refreshDaemonState()
            if self.pipeline.installSnapshot?.isRunning == false { return }
            guard clock.now < deadline else { throw VoiceInstallCancelError.stillRunning }
            try await Task.sleep(for: .milliseconds(500))
        }
    }

    func settled() async -> VoiceInstallOutcome? {
        for await outcome in Observations({ self.outcome }) where outcome != .installing {
            if case .failed = outcome, case .failed = self.pipeline.sttStatus {
                do {
                    try await self.cancel()
                } catch {
                    return .failed(L10n.tr("setup.voice.cancel_failed_format", error.localizedDescription))
                }
            }
            return outcome
        }
        return nil
    }

    func monitorIfInstalling() {
        guard self.pipeline.installSnapshot?.isRunning == true else { return }
        self.monitor()
    }

    private func monitor() {
        let selection = EndpointStore.shared.selection
        if self.monitorTask != nil, self.monitorSelection == selection { return }
        self.monitorTask?.cancel()
        self.monitorSelection = selection
        self.monitorTask = Task { [weak self] in
            guard let self else { return }
            defer {
                if self.monitorSelection == selection {
                    self.monitorTask = nil
                    self.monitorSelection = nil
                }
            }
            let pipeline = self.pipeline
            while !Task.isCancelled, EndpointStore.shared.selection == selection {
                let revision = pipeline.installSnapshotRevision
                await pipeline.refreshDaemonState()
                guard !Task.isCancelled, EndpointStore.shared.selection == selection else { return }
                if revision != pipeline.installSnapshotRevision, pipeline.installSnapshot?.isTerminal == true { return }
                do { try await Task.sleep(for: .milliseconds(500)) } catch { return }
            }
        }
    }

    private func prepareSelectedTts() async throws {
        self.isPreparingTts = true
        defer { self.isPreparingTts = false }
        switch VoiceTtsPreference.shared.backend {
        case .serverPocket:
            try await self.startDaemonTts()
        case .clientSupertonic:
            try await self.prepareSupertonic()
        }
    }

    private func startDaemonTts() async throws {
        let pipeline = self.pipeline
        pipeline.synchronizeDaemonSelection()
        let selection = pipeline.daemonStateSelection
        try await DaemonClient(endpoint: selection.endpoint).startVoiceInstall(kind: VoiceWire.modelKindTts)
        guard EndpointStore.shared.selection == selection else { return }
        if let refresh = pipeline.refreshDaemonTask { await refresh.value }
        self.staleSnapshotRevision = pipeline.installSnapshotRevision
        await pipeline.refreshDaemonState()
        guard EndpointStore.shared.selection == selection else { return }
        self.monitor()
    }

    private func prepareSupertonic() async throws {
        defer { self.supertonicInstalled = FluidAudioSupertonicModelPresence.isInstalled() }
        try await self.pipeline.clientTtsEngine.prepare()
    }

    private var pocketStatus: VoiceModelStatus {
        guard let snapshot = self.pipeline.installSnapshot,
              let progress = snapshot.models.first(where: { $0.kind == VoiceWire.modelKindTts })
        else { return .unknown }
        if snapshot.ttsReady { return .installed }
        if progress.status.starts(with: "downloading") || self.isPreparingTts || self.awaitsDaemonStatus {
            return .downloading(
                percent: progress.percent,
                bytesDownloaded: progress.bytesDownloaded,
                bytesTotal: progress.bytesTotal
            )
        }
        if snapshot.status == .failed {
            return .failed(snapshot.error ?? L10n.tr("setup.voice.poll_failure"))
        }
        return .missing
    }

    private func sttStatus<P: FluidAudioPresence>(_ presence: P.Type, english: Bool) -> VoiceModelStatus {
        let pipeline = self.pipeline
        if let downloading = pipeline.sttDownloadLanguage, (downloading == "en") == english {
            return .downloading(percent: pipeline.sttLoadProgress?.percent)
        }
        if (pipeline.activeSttLanguage == "en") == english {
            switch pipeline.sttStatus {
            case .ready: return .installed
            case let .failed(message): return .failed(message)
            case .downloading: return .downloading(percent: nil)
            case .notLoaded: break
            }
        }
        return P.isInstalled() ? .installed : .missing
    }
}

enum VoiceInstallCancelError: LocalizedError {
    case stillRunning

    var errorDescription: String? {
        L10n.tr("setup.voice.cancel_still_running")
    }
}
