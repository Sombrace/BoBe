@preconcurrency import AudioToolbox
@preconcurrency import AVFoundation
import Foundation
import Observation
import OSLog

nonisolated private let logger = Logger(subsystem: "com.bobe.app", category: "VoicePipeline")

@Observable
public final class VoicePipeline {
    public enum State: Equatable {
        case idle
        case connecting
        case listening
        case capturing
        case thinking
        case speaking
        case cancelling
        case failed(String)
    }

    public static let shared = VoicePipeline()

    public internal(set) var state: State = .idle
    public internal(set) var lastError: String?
    public internal(set) var terminalServerErrorCode: String?
    public internal(set) var sessionRestartError: String?

    public enum SttStatus: Equatable {
        case notLoaded
        case downloading
        case ready
        case failed(String)
    }

    public internal(set) var sttStatus: SttStatus = .notLoaded
    private(set) var sttLoadProgress: VoiceSttLoadProgress?
    private(set) var sttDownloadLanguage: String?

    var permission: AVAuthorizationStatus =
        AVCaptureDevice.authorizationStatus(for: .audio)

    var installSnapshot: VoiceInstallSnapshot?
    var installSnapshotRevision: UInt64 = 0
    var daemonSttReady = false
    @ObservationIgnored var sttStatusLanguage = "en"

    var refreshDaemonTask: Task<Void, Never>?
    var daemonStateSelection = EndpointStore.shared.selection
    @ObservationIgnored private var settingsObservation: Task<Void, Never>?
    @ObservationIgnored private var sessionSettingsObservation: Task<Void, Never>?
    @ObservationIgnored var negotiatedSettings: VoiceSessionSettings?
    @ObservationIgnored var renegotiationTask: Task<Void, Never>?
    @ObservationIgnored var renegotiating = false

    public internal(set) var inputLevel: Float = 0
    public internal(set) var ttsOutputLevel: Float = 0
    public internal(set) var isTtsAudible: Bool = false

    let audibleOnThreshold: Float = 0.04
    let audibleOffThreshold: Float = 0.01
    let audibleHangoverMs: Int = 350
    var audibleHangoverTask: Task<Void, Never>?

    public internal(set) var partialTranscript: String = ""

    let playbackSampleRate: Double = Double(VoiceWire.ttsOutputSampleRate)

    let ttsOutputFormat: AVAudioFormat?

    static let jsonEncoder = JSONEncoder()
    static let jsonDecoder = JSONDecoder()

    let audioEngine = AVAudioEngine()
    let playerNode = AVAudioPlayerNode()

    var recognizers: [String: AppleSpeechStt] = [:]

    var sessionSttLanguage: String?
    var sessionTtsBackend: VoiceTtsBackend?
    var sessionVoicePersona: String?
    var sessionVoiceSpeed: Float?
    var sessionSpeechPlan: SpeechPlanWire?
    var sessionSpeechOffer: SpeechOfferWire?
    var handshakeComplete = false

    var loadedSttLanguages: Set<String> = []
    var sttLoaded: Bool {
        self.loadedSttLanguages.contains(self.effectiveLanguage)
    }

    var pendingTurnId: String?
    var pendingPartialForServer: String?
    var partialSendTask: Task<Void, Never>?
    @ObservationIgnored var heldUtterance: String?
    @ObservationIgnored var heldUtteranceCount = 0
    @ObservationIgnored var heldUtteranceTask: Task<Void, Never>?

    var decoder: OpusDecoder?
    var fillerDecoder: OpusDecoder?
    var scheduledChunks: [ScheduledTtsChunk] = []
    var nextScheduleFrame: AVAudioFramePosition = 0
    var sampleTimeBase: AVAudioFramePosition = 0
    var turnFrameBase: AVAudioFramePosition = 0
    @ObservationIgnored var turnAudioStartFrame: AVAudioFramePosition?
    @ObservationIgnored var nextReplyChunkId: UInt64 = 0
    @ObservationIgnored var overlapPlayheadChunk: UInt64?
    @ObservationIgnored var serverDrainTask: Task<Void, Never>?
    @ObservationIgnored var replyPlayback: ReplyPlayback?
    @ObservationIgnored var phaseTracker = VoicePhaseTracker()

    let inputProcessor = VoiceInputProcessor()
    let audioLifecycle = AudioLifecycle()
    var audioInputContinuation: AsyncStream<CapturedVoiceAudio>.Continuation?
    var audioInputTask: Task<Void, Never>?
    var droppedAudioInputChunks = 0
    let systemVoice = SystemVoice()
    var clientTtsQueue: ClientTtsSentenceQueue?
    var clientTtsTask: Task<Void, Never>?
    var clientTtsTurnId: String?
    @ObservationIgnored var cancelledClientTtsTurnId: String?
    var clientTtsActive = false
    var clientTtsFirstAudioSent = false
    var pendingServerListening = false
    public private(set) var muted = false

    var task: URLSessionWebSocketTask?
    var receiveTask: Task<Void, Never>?
    var handshakeTask: Task<Void, Never>?
    var connectionGeneration: UInt64 = 0
    var captureStreamId: UInt32?
    var captureSequence: UInt32 = 0
    var captureCreditFrames: UInt32 = 0
    var captureSamples = VoiceSampleBuffer()
    var lastPlaybackAckMs: UInt64 = 0
    var lastPlaybackAckChunkId: UInt64 = 0
    var playbackAckTask: Task<Void, Never>?
    var playbackAckGeneration: UInt64 = 0
    var sessionId: String = ""
    var keepaliveTask: Task<Void, Never>?
    var ttsLevelDecayTask: Task<Void, Never>?
    var lastTtsRmsAt: ContinuousClock.Instant?
    private var welcomeObserver: NotificationCenter.ObservationToken?
    private var engineConfigurationObserver: NSObjectProtocol?
    let keepalivePingInterval: Duration = .seconds(25)

    var isWarm = false
    var inputVoiceProcessingEnabled = false
    var outputVoiceProcessingEnabled = false
    var inputTapInstalled = false
    var playerTapInstalled = false

    var inputDeviceListenerRegistered = false
    var outputDeviceListenerRegistered = false
    var deviceChangeReconfigureTask: Task<Void, Never>?
    var pendingAudioDeviceRebuild = false

    let bargeInRmsDbfs: Float = -40
    let bargeInFramesNeeded: Int = 3
    var bargeInCount: Int = 0
    var bargeInSent: Bool = false
    @ObservationIgnored var playbackDucked = false
    @ObservationIgnored var lastOverlapAt = ContinuousClock.now
    @ObservationIgnored var duckTask: Task<Void, Never>?

    private init() {
        self.ttsOutputFormat = AVAudioFormat(
            commonFormat: .pcmFormatInt16,
            sampleRate: self.playbackSampleRate,
            channels: 1,
            interleaved: false
        )
        self.audioEngine.attach(self.playerNode)
        if let format = self.ttsOutputFormat {
            if #available(macOS 27, *) {
                do {
                    try self.audioEngine.connectNode(self.playerNode, to: self.audioEngine.mainMixerNode, format: format)
                } catch {
                    logger.error("voice player connection failed: \(error.localizedDescription, privacy: .public)")
                }
            } else {
                self.audioEngine.connect(self.playerNode, to: self.audioEngine.mainMixerNode, format: format)
            }
        }
        self.welcomeObserver = NotificationCenter.default.addObserver(
            for: WelcomeCompletedMessage.self
        ) { [weak self] _ in
            Task(name: "Refresh voice after welcome") { [weak self] in
                await self?.refreshDaemonState()
            }
        }
        self.engineConfigurationObserver = NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange,
            object: self.audioEngine,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.handleAudioEngineConfigurationChange()
            }
        }
        self.registerApplicationMuteHandler()
        self.observeVoiceSettings()
    }

    private func observeVoiceSettings() {
        self.settingsObservation = Task(name: "Observe voice settings") { [weak self] in
            for await _ in Observations({ SettingsStore.shared.settings }) {
                self?.reconcileVoiceSettings()
            }
        }
        self.sessionSettingsObservation = Task(name: "Observe voice session settings") { [weak self] in
            for await desired in Observations({ VoiceSessionSettings.current() }) {
                self?.reconcileSessionSettings(desired)
            }
        }
    }

    public func prewarm() async -> Bool {
        await self.bootstrapSttPresence()
        if self.sttStatus == .ready, !self.sttLoaded {
            await self.ensureSttLoaded()
        } else if self.sttStatus == .notLoaded {
            Task(name: "Speech recognition install") { await self.ensureSttLoaded() }
        }
        guard !self.isWarm else { return true }
        do {
            try await self.configureEngine()
            self.isWarm = true
            return true
        } catch is CancellationError {
            return false
        } catch {
            self.lastError = "voice prewarm: \(error.localizedDescription)"
            logger.error("prewarm failed: \(error.localizedDescription, privacy: .public)")
            return false
        }
    }

    public func prewarmSttOnly() async {
        await self.bootstrapSttPresence()
        if self.sttStatus == .ready, !self.sttLoaded {
            Task(name: "Speech recognition prewarm") { [weak self] in await self?.ensureSttLoaded() }
        }
    }

    public func bootstrapSttPresence() async {
        guard self.sttStatus != .ready || self.sttStatusLanguage != self.activeSttLanguage,
              self.sttStatus != .downloading
        else { return }
        await self.refreshSttState()
    }

    public func ensureSttLoaded(for requestedLanguage: String? = nil) async {
        let language = requestedLanguage ?? self.activeSttLanguage
        if self.loadedSttLanguages.contains(language) {
            return
        }
        let isActiveLanguage = language == self.activeSttLanguage
        if isActiveLanguage, self.sttStatus != .ready, self.sttStatus != .downloading {
            self.sttStatus = .downloading
        }
        self.sttDownloadLanguage = language
        self.sttLoadProgress = VoiceSttLoadProgress(fractionCompleted: 0)
        let onPartial: @Sendable (String) async -> Void = { [weak self] text in
            await self?.handleSttPartial(text)
        }
        let onEou: @Sendable (String) async -> Void = { [weak self] text in
            await self?.handleSttEou(text)
        }
        let onFailure: @Sendable (String) async -> Void = { [weak self] message in
            await self?.handleSttFailure(message, language: language)
        }
        let onProgress: @Sendable (VoiceSttLoadProgress) -> Void = { [weak self] progress in
            Task { @MainActor in
                guard self?.sttDownloadLanguage == language else { return }
                self?.sttLoadProgress = progress
            }
        }
        let recognizer = self.recognizer(for: language)
        let timeout = Duration.seconds(30 * 60)
        do {
            try await withVoiceLoadTimeout(timeout) {
                try await recognizer.load(
                    onPartial: onPartial,
                    onEou: onEou,
                    onFailure: onFailure,
                    onProgress: onProgress
                )
            }
            self.loadedSttLanguages.insert(language)
            if isActiveLanguage {
                self.sttStatus = .ready
            }
            if self.sttDownloadLanguage == language {
                self.sttDownloadLanguage = nil
                self.sttLoadProgress = nil
            }
            logger.info("Apple Speech ready language=\(language, privacy: .public)")
        } catch is VoiceLoadTimeout {
            await recognizer.cancelLoading()
            let msg = "Speech recognition setup timed out after \(timeout)"
            self.lastError = msg
            if isActiveLanguage {
                self.sttStatus = .failed(msg)
            }
            self.sttDownloadLanguage = nil
            self.sttLoadProgress = nil
            logger.error("Apple Speech setup timed out language=\(language, privacy: .public)")
        } catch is CancellationError {
            self.sttDownloadLanguage = nil
            self.sttLoadProgress = nil
            if isActiveLanguage {
                await self.refreshSttState()
            }
        } catch {
            let msg = error.localizedDescription
            self.lastError = "STT load: \(msg)"
            if isActiveLanguage {
                self.sttStatus = .failed(msg)
            }
            self.sttDownloadLanguage = nil
            self.sttLoadProgress = nil
            logger.error(
                "Apple Speech setup failed language=\(language, privacy: .public): \(msg, privacy: .public)"
            )
        }
    }

    public func cancelSttLoading() async {
        let language = self.sttDownloadLanguage ?? self.activeSttLanguage
        await self.recognizer(for: language).cancelLoading()
        self.sttDownloadLanguage = nil
        self.sttLoadProgress = nil
        if language == self.activeSttLanguage {
            await self.refreshSttState()
        }
    }

    private static func mintTurnId() -> String {
        "voice_\(UUID().uuidString.lowercased().replacingOccurrences(of: "-", with: ""))"
    }

    private func handleSttPartial(_ fragment: String) {
        guard !fragment.isEmpty else { return }
        if self.pendingTurnId == nil {
            self.pendingTurnId = Self.mintTurnId()
        }
        let turnId = self.pendingTurnId ?? ""
        self.heldUtteranceTask?.cancel()
        self.heldUtteranceTask = nil
        let text = TurnCompletion.joined(self.heldUtterance, fragment)
        self.partialTranscript = text
        self.pendingPartialForServer = text
        guard self.partialSendTask == nil else { return }
        self.partialSendTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(80))
            guard let self, !Task.isCancelled, self.pendingTurnId == turnId else { return }
            let latest = self.pendingPartialForServer
            self.pendingPartialForServer = nil
            self.partialSendTask = nil
            if let latest {
                await self.sendClient(.transcriptPartial(turnId: turnId, text: latest))
            }
        }
    }

    private func handleSttFailure(_ message: String, language: String) {
        self.loadedSttLanguages.remove(language)
        guard self.effectiveLanguage == language else { return }
        self.sttStatus = .failed(message)
        self.lastError = "STT runtime: \(message)"
        if self.task != nil {
            self.tearDownConnection(
                finalState: .failed(message),
                sendAbort: false
            )
        }
    }

    private func handleSttEou(_ text: String) async {
        let fragment = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !fragment.isEmpty else { return }
        guard self.task != nil else {
            logger.debug("voice.eou_after_disconnect_ignored")
            return
        }
        let utterance = TurnCompletion.joined(self.heldUtterance, fragment)
        if self.state != .speaking,
           self.heldUtteranceCount < TurnCompletion.maximumHolds,
           TurnCompletion.looksUnfinished(utterance) {
            self.holdUtterance(utterance)
            return
        }
        await self.finishUtterance(utterance)
    }

    private func holdUtterance(_ utterance: String) {
        if self.pendingTurnId == nil {
            self.pendingTurnId = Self.mintTurnId()
        }
        self.heldUtterance = utterance
        self.heldUtteranceCount += 1
        self.partialTranscript = utterance
        self.heldUtteranceTask?.cancel()
        self.heldUtteranceTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(TurnCompletion.graceMilliseconds))
            guard let self, !Task.isCancelled, let held = self.heldUtterance else { return }
            self.heldUtteranceTask = nil
            await self.finishUtterance(held)
        }
    }

    private func finishUtterance(_ utterance: String) async {
        let turnId = self.pendingTurnId ?? Self.mintTurnId()
        self.pendingTurnId = nil
        self.clearHeldUtterance()
        self.partialSendTask?.cancel()
        self.partialSendTask = nil
        self.pendingPartialForServer = nil
        self.partialTranscript = ""
        await self.sendClient(.transcriptFinal(turnId: turnId, text: utterance))
    }

    func clearHeldUtterance() {
        self.heldUtteranceTask?.cancel()
        self.heldUtteranceTask = nil
        self.heldUtterance = nil
        self.heldUtteranceCount = 0
    }

    @discardableResult
    public func setMuted(_ shouldMute: Bool) async -> Bool {
        guard self.muted != shouldMute else { return true }
        do {
            try AVAudioApplication.shared.setInputMuted(shouldMute)
        } catch {
            self.lastError = "application microphone mute: \(error.localizedDescription)"
            logger.warning(
                "application microphone mute failed: \(error.localizedDescription, privacy: .public)"
            )
            return false
        }
        self.applyLogicalMute(shouldMute)
        return self.muted == shouldMute
    }

    private func registerApplicationMuteHandler() {
        do {
            try AVAudioApplication.shared.setInputMuteStateChangeHandler { [weak self] shouldMute in
                let apply = {
                    MainActor.assumeIsolated {
                        self?.applyLogicalMute(shouldMute)
                    }
                }
                if Thread.isMainThread {
                    apply()
                } else {
                    DispatchQueue.main.sync(execute: apply)
                }
                return true
            }
        } catch {
            logger.warning(
                "application microphone mute handler unavailable: \(error.localizedDescription, privacy: .public)"
            )
        }
    }

    private func applyLogicalMute(_ shouldMute: Bool) {
        guard self.muted != shouldMute else { return }
        self.muted = shouldMute
        if shouldMute {
            self.inputLevel = 0
            self.bargeInCount = 0
            self.bargeInSent = false
            self.captureSamples.removeAll()
            self.partialTranscript = ""
            self.clearHeldUtterance()
            self.pendingPartialForServer = nil
            self.partialSendTask?.cancel()
            self.partialSendTask = nil
            Task.immediate { try? await self.activeStt.reset() }
        }
        Task.immediate { await self.sendClient(.control(action: shouldMute ? .mute : .unmute)) }
    }
}
