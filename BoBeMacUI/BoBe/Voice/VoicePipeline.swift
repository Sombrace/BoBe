@preconcurrency import AudioToolbox
@preconcurrency import AVFoundation
import FluidAudio
import Foundation
import Observation
import OSLog

private let logger = Logger(subsystem: "com.bobe.app", category: "VoicePipeline")

@MainActor
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
    var voiceEnabled: Bool?
    var daemonSttReady = false

    var showPartialCaption: Bool = true
    var voicePersona: String?
    var voiceSpeed: Float?

    var refreshDaemonTask: Task<Void, Never>?
    var daemonStateSelection = EndpointStore.shared.selection
    var installPollTask: Task<Void, Never>?
    var installSnapshotRevision: UInt64 = 0

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

    let parakeetStt = FluidAudioStt()
    let nemotronStt = FluidAudioNemotronStt()
    var appleSttByLanguage: [String: AppleSpeechStt] = [:]
    var activeSttLanguage: String = "en"

    var sessionSttLanguage: String?
    var sessionSttEngineId: String?
    var sessionTtsBackend: VoiceTtsBackend?
    var sessionVoicePersona: String?
    var sessionVoiceSpeed: Float?
    var sessionSpeechPlan: SpeechPlanWire?
    var sessionSpeechOffer: SpeechOfferWire?
    var handshakeComplete = false

    var loadedSttKeys: Set<String> = []
    var effectiveSttEngineId: String {
        self.sessionSttEngineId ?? self.preferredSttEngineId(for: self.activeSttLanguage)
    }
    var sttLoaded: Bool {
        self.loadedSttKeys.contains(
            Self.sttReadinessKey(engineId: self.effectiveSttEngineId, language: self.effectiveLanguage)
        )
    }

    var pendingTurnId: String?
    var pendingPartialForServer: String?
    var partialSendTask: Task<Void, Never>?
    @ObservationIgnored var heldUtterance: String?
    @ObservationIgnored var heldUtteranceCount = 0
    @ObservationIgnored var heldUtteranceTask: Task<Void, Never>?

    var decoder: OpusDecoder?
    var scheduledChunks: [ScheduledTtsChunk] = []
    var nextScheduleFrame: AVAudioFramePosition = 0
    var sampleTimeBase: AVAudioFramePosition = 0
    var turnFrameBase: AVAudioFramePosition = 0
    @ObservationIgnored var turnAudioStartFrame: AVAudioFramePosition?
    @ObservationIgnored var nextReplyChunkId: UInt64 = 0
    @ObservationIgnored var overlapPlayheadChunk: UInt64?
    @ObservationIgnored var serverDrainTask: Task<Void, Never>?

    let inputProcessor = VoiceInputProcessor()
    var audioInputContinuation: AsyncStream<CapturedVoiceAudio>.Continuation?
    var audioInputTask: Task<Void, Never>?
    var droppedAudioInputChunks = 0
    let clientTtsEngine = ClientTtsEngine()
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
    private var sessionId: String = ""
    var keepaliveTask: Task<Void, Never>?
    var ttsLevelDecayTask: Task<Void, Never>?
    var lastTtsRmsAt: ContinuousClock.Instant?
    private var notificationObservers: [NSObjectProtocol] = []
    let keepalivePingIntervalNs: UInt64 = 25_000_000_000

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
    @ObservationIgnored var lastOverlapAt: Date = .distantPast
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
            self.audioEngine.connect(
                self.playerNode,
                to: self.audioEngine.mainMixerNode,
                format: format
            )
        }
        let token = NotificationCenter.default.addObserver(
            forName: .bobeWelcomeCompleted,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                await self?.refreshDaemonState()
            }
        }
        self.notificationObservers.append(token)
        let engineToken = NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange,
            object: self.audioEngine,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.handleAudioEngineConfigurationChange()
            }
        }
        self.notificationObservers.append(engineToken)
        self.registerApplicationMuteHandler()
    }


    public func prewarm() async -> Bool {
        self.bootstrapSttPresence()
        if VoiceSttPreferenceStore.shared.preference.usesEndpointEngine,
           self.sttStatus == .ready,
           !self.sttLoaded {
            await self.ensureSttLoaded()
        }
        guard !self.isWarm else { return true }
        do {
            try self.configureEngine()
            self.isWarm = true
            return true
        } catch {
            self.lastError = "voice prewarm: \(error.localizedDescription)"
            logger.error("prewarm failed: \(error.localizedDescription, privacy: .public)")
            return false
        }
    }

    public func prewarmSttOnly() async {
        guard VoiceSttPreferenceStore.shared.preference.usesEndpointEngine else { return }
        self.bootstrapSttPresence()
        if self.sttStatus == .ready, !self.sttLoaded {
            Task { [weak self] in await self?.ensureSttLoaded() }
        }
    }

    public func bootstrapSttPresence() {
        if VoiceSttPreferenceStore.shared.preference == .daemon {
            self.sttStatus = self.daemonSttReady ? .ready : .notLoaded
            return
        }
        if self.sttStatus == .ready {
            return
        }
        self.sttStatus = self.activeModelIsInstalled ? .ready : .notLoaded
    }

    public func ensureSttLoaded(for requestedLanguage: String? = nil) async {
        guard VoiceSttPreferenceStore.shared.preference.usesEndpointEngine else { return }
        let language = requestedLanguage ?? self.activeSttLanguage
        let engineId = self.preferredSttEngineId(for: language)
        let readinessKey = Self.sttReadinessKey(engineId: engineId, language: language)
        if self.loadedSttKeys.contains(readinessKey) {
            return
        }
        let isActiveLanguage = language == self.activeSttLanguage
        if isActiveLanguage, self.sttStatus != .ready, self.sttStatus != .downloading {
            self.sttStatus = .downloading
        }
        self.sttDownloadLanguage = language
        self.sttLoadProgress = VoiceSttLoadProgress(
            fractionCompleted: 0,
            phase: .listing
        )
        let onPartial: @Sendable (String) async -> Void = { [weak self] text in
            await self?.handleSttPartial(text)
        }
        let onEou: @Sendable (String) async -> Void = { [weak self] text in
            await self?.handleSttEou(text)
        }
        let onFailure: @Sendable (String) async -> Void = { [weak self] message in
            await self?.handleSttFailure(
                message,
                engineId: engineId,
                language: language
            )
        }
        let onProgress: @Sendable (VoiceSttLoadProgress) -> Void = { [weak self] progress in
            Task { @MainActor in
                guard self?.sttDownloadLanguage == language else { return }
                self?.sttLoadProgress = progress
            }
        }
        let engine = self.sttEngine(engineId: engineId, language: language)
        let timeoutSeconds: TimeInterval = 30 * 60
        do {
            try await withVoiceLoadTimeout(seconds: timeoutSeconds) {
                try await engine.loadModels(
                    onPartial: onPartial,
                    onEou: onEou,
                    onFailure: onFailure,
                    onProgress: onProgress
                )
            }
            self.loadedSttKeys.insert(readinessKey)
            if isActiveLanguage {
                self.sttStatus = .ready
            }
            if self.sttDownloadLanguage == language {
                self.sttDownloadLanguage = nil
                self.sttLoadProgress = nil
            }
            logger.info(
                "endpoint STT loaded engine=\(engineId, privacy: .public) language=\(language, privacy: .public)"
            )
        } catch is VoiceLoadTimeout {
            await engine.cancelLoading()
            let msg = "STT load timed out after \(Int(timeoutSeconds))s"
            self.lastError = msg
            if isActiveLanguage {
                self.sttStatus = .failed(msg)
            }
            self.sttDownloadLanguage = nil
            self.sttLoadProgress = nil
            logger.error(
                "endpoint STT load timed out engine=\(engineId, privacy: .public) language=\(language, privacy: .public)"
            )
        } catch is CancellationError {
            if isActiveLanguage {
                self.sttStatus = self.loadedSttKeys.contains(readinessKey)
                    || (engineId != VoiceSttEngineId.appleSpeechAnalyzer
                        && self.modelIsInstalled(for: language)) ? .ready : .notLoaded
            }
            self.sttDownloadLanguage = nil
            self.sttLoadProgress = nil
        } catch {
            let msg = error.localizedDescription
            self.lastError = "STT load: \(msg)"
            if isActiveLanguage {
                self.sttStatus = .failed(msg)
            }
            self.sttDownloadLanguage = nil
            self.sttLoadProgress = nil
            logger.error(
                "endpoint STT load failed engine=\(engineId, privacy: .public) language=\(language, privacy: .public): \(msg, privacy: .public)"
            )
        }
    }

    public func prepareSelectedTts() async throws {
        switch VoiceTtsPreference.shared.backend {
        case .serverPocket:
            try await self.startDaemonTtsInstall()
        case .clientSupertonic:
            try await self.clientTtsEngine.prepare()
        }
    }

    public func cancelSttLoading() async {
        let language = self.sttDownloadLanguage ?? self.activeSttLanguage
        let engineId = self.preferredSttEngineId(for: language)
        await self.sttEngine(engineId: engineId, language: language).cancelLoading()
        self.sttDownloadLanguage = nil
        self.sttLoadProgress = nil
        if language == self.activeSttLanguage {
            let key = Self.sttReadinessKey(engineId: engineId, language: language)
            self.sttStatus = self.loadedSttKeys.contains(key)
                || (engineId != VoiceSttEngineId.appleSpeechAnalyzer
                    && self.modelIsInstalled(for: language)) ? .ready : .notLoaded
        }
    }

    public func unloadSttModel(for language: String) async {
        let engineId = self.preferredSttEngineId(for: language)
        await self.sttEngine(engineId: engineId, language: language).cleanup()
        self.loadedSttKeys.remove(
            Self.sttReadinessKey(engineId: engineId, language: language)
        )
        if engineId == VoiceSttEngineId.appleSpeechAnalyzer {
            self.appleSttByLanguage[language] = nil
        }
        if self.activeSttLanguage == language {
            self.sttStatus = .notLoaded
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
        self.partialSendTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(80))
            guard let self, !Task.isCancelled else { return }
            let latest = self.pendingPartialForServer
            self.pendingPartialForServer = nil
            self.partialSendTask = nil
            if let latest {
                await self.sendClient(.transcriptPartial(turnId: turnId, text: latest))
            }
        }
    }

    private func handleSttFailure(
        _ message: String,
        engineId: String,
        language: String
    ) {
        let readinessKey = Self.sttReadinessKey(
            engineId: engineId,
            language: language
        )
        self.loadedSttKeys.remove(readinessKey)
        guard self.effectiveSttEngineId == engineId,
              self.effectiveLanguage == language
        else { return }
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
        self.heldUtteranceTask = Task { @MainActor [weak self] in
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

    public func connect(daemonBaseURL: URL) {
        self.lastError = nil
        self.terminalServerErrorCode = nil

        guard let wsURL = voiceWsEndpoint(from: daemonBaseURL) else {
            self.lastError = "invalid daemon URL for WS"
            logger.error("invalid daemon URL for WS")
            return
        }

        self.sessionSttLanguage = self.activeSttLanguage
        self.sessionSttEngineId = self.preferredSttEngineId(for: self.activeSttLanguage)
        self.sessionTtsBackend = VoiceTtsPreference.shared.backend
        self.sessionVoicePersona = self.voicePersona
        self.sessionVoiceSpeed = self.voiceSpeed
        let language = self.activeSttLanguage
        if language != "en" {
            Task { [weak self] in await self?.nemotronStt.setLanguage(language) }
        }

        self.state = .connecting
        var request = URLRequest(url: wsURL)
        DaemonConfig.endpoint.authorize(&request)
        request.setValue(VoiceWire.subprotocolV2, forHTTPHeaderField: "Sec-WebSocket-Protocol")
        self.connectionGeneration &+= 1
        let generation = self.connectionGeneration
        self.handshakeTask?.cancel()
        let webSocketSession = DaemonTransportPolicy.session(
            for: DaemonConfig.endpoint,
            purpose: .realtimeWebSocket
        )
        let newTask = webSocketSession.webSocketTask(with: request)
        self.task = newTask
        newTask.resume()
        self.startReceiveLoop(socket: newTask, generation: generation)
        self.startKeepalive()

        self.sessionId = "voice-\(Int(Date.now.timeIntervalSince1970))"
        let sid = self.sessionId
        let voiceId = self.sessionVoicePersona
        let speed = self.sessionVoiceSpeed
        let ttsBackend = self.sessionTtsBackend ?? .serverPocket
        self.handshakeTask = Task { @MainActor [weak self, weak newTask] in
            guard let self,
                  let newTask,
                  self.task === newTask,
                  self.connectionGeneration == generation,
                  !Task.isCancelled
            else { return }
            if ttsBackend == .clientSupertonic {
                do {
                    try await self.clientTtsEngine.prepare()
                } catch {
                    let reason = "Supertonic prepare: \(error.localizedDescription)"
                    self.lastError = reason
                    self.tearDownConnection(finalState: .failed(reason), sendAbort: false)
                    return
                }
            }
            guard await self.prewarm() else {
                let reason = self.lastError ?? "voice audio setup failed"
                self.tearDownConnection(finalState: .failed(reason), sendAbort: false)
                return
            }
            guard !Task.isCancelled,
                  self.task === newTask,
                  self.connectionGeneration == generation
            else { return }
            let speechOffer = self.makeSpeechOffer(
                language: language,
                ttsBackend: ttsBackend,
                voiceId: voiceId
            )
            self.sessionSpeechOffer = speechOffer
            await self.sendClient(.hello(
                sessionId: sid,
                voiceId: voiceId,
                speed: speed,
                speechOffer: speechOffer
            ))
            if self.muted {
                await self.sendClient(.control(action: .mute))
            }
            guard !Task.isCancelled,
                  self.task === newTask,
                  self.connectionGeneration == generation
            else { return }
            let negotiationAttempts = DaemonConfig.endpoint.isLoopbackRuntime ? 20 : 80
            for _ in 0 ..< negotiationAttempts {
                if self.handshakeComplete || self.state != .connecting { return }
                try? await Task.sleep(for: .milliseconds(100))
                if Task.isCancelled { return }
            }
            guard self.task === newTask,
                  self.connectionGeneration == generation,
                  self.state == .connecting
            else { return }
            let reason = self.lastError ?? "voice negotiation timed out"
            self.lastError = reason
            self.tearDownConnection(finalState: .failed(reason), sendAbort: false)
        }
    }

    private func makeSpeechOffer(
        language: String,
        ttsBackend: VoiceTtsBackend,
        voiceId: String?
    ) -> SpeechOfferWire {
        let defaults = UserDefaults.standard
        let endpointId = defaults.string(forKey: EndpointAttachService.surfaceIdKey)
            .flatMap(UUID.init(uuidString:)) ?? UUID()
        if defaults.string(forKey: EndpointAttachService.surfaceIdKey) == nil {
            defaults.set(endpointId.uuidString, forKey: EndpointAttachService.surfaceIdKey)
        }
        let endpointStt: [SpeechEngineOfferWire]
        if VoiceSttPreferenceStore.shared.preference == .daemon || !self.sttLoaded {
            endpointStt = []
        } else {
            endpointStt = [SpeechEngineOfferWire(
                id: self.effectiveSttEngineId,
                languages: [language],
                partials: true,
                eou: true,
                voices: [],
                sampleRates: [],
                ready: true
            )]
        }
        let endpointTts = ttsBackend == .clientSupertonic ? [SpeechEngineOfferWire(
            id: VoiceTtsBackend.clientSupertonic.rawValue,
            languages: [language],
            partials: false,
            eou: false,
            voices: [Self.supertonicVoiceId(for: voiceId)],
            sampleRates: [UInt32(VoiceWire.ttsOutputSampleRate)],
            ready: true
        )] : []
        return SpeechOfferWire(
            endpointId: endpointId,
            endpointKind: "mac_app",
            language: language,
            playbackRates: [UInt32(VoiceWire.ttsOutputSampleRate)],
            endpointStt: endpointStt,
            endpointTts: endpointTts,
            audioUplink: true,
            overrides: SpeechOverridesWire(
                stt: VoiceSttPreferenceStore.shared.preference.placement,
                tts: ttsBackend == .clientSupertonic ? .endpoint : .daemon
            ),
            daemonIsNonLoopback: !DaemonEndpoint.isLoopbackHost(
                DaemonConfig.endpoint.baseURL.host ?? ""
            ),
            remoteAudioConsent: RemoteSpeechConsent.shared.currentAuthorityEnabled
        )
    }

    static func supertonicVoiceId(for canonicalVoiceId: String?) -> String {
        let normalized = canonicalVoiceId?.lowercased() ?? ""
        return normalized.hasPrefix("am_") || normalized.hasPrefix("bm_")
            ? "supertonic_m1"
            : "supertonic_f1"
    }

    public func disconnect() {
        self.tearDownConnection(finalState: .idle, sendAbort: true)
    }

    func tearDownConnection(finalState: State, sendAbort: Bool) {
        self.stopKeepalive()
        self.receiveTask?.cancel()
        self.receiveTask = nil
        self.handshakeTask?.cancel()
        self.handshakeTask = nil
        self.connectionGeneration &+= 1
        let oldTask = self.task
        self.task = nil
        self.state = finalState
        self.inputLevel = 0
        self.partialTranscript = ""
        self.clearHeldUtterance()
        self.pendingPartialForServer = nil
        self.partialSendTask?.cancel()
        self.partialSendTask = nil
        Task {
            if let oldTask {
                if sendAbort,
                   let data = try? Self.jsonEncoder.encode(
                       ClientVoiceMessage.control(action: .abort)
                   ),
                   let text = String(data: data, encoding: .utf8) {
                    try? await oldTask.send(.string(text))
                }
                oldTask.cancel(with: .normalClosure, reason: nil)
            }
        }
        self.tearDownAudio()
        self.sessionSttLanguage = nil
        self.sessionSttEngineId = nil
        self.sessionTtsBackend = nil
        self.sessionVoicePersona = nil
        self.sessionVoiceSpeed = nil
        self.sessionSpeechPlan = nil
        self.sessionSpeechOffer = nil
        self.handshakeComplete = false
        self.captureStreamId = nil
        self.captureSequence = 0
        self.captureCreditFrames = 0
        self.captureSamples.removeAll()
        self.playbackAckTask?.cancel()
        self.playbackAckTask = nil
        self.playbackAckGeneration &+= 1
        self.lastPlaybackAckMs = 0
        self.lastPlaybackAckChunkId = 0
        self.serverDrainTask?.cancel()
        self.serverDrainTask = nil
        self.resetReplyPosition()
    }

    public func toggle(daemonBaseURL: URL) async {
        switch self.state {
        case .idle, .failed:
            self.connect(daemonBaseURL: daemonBaseURL)
            while self.state == .connecting, self.task != nil {
                try? await Task.sleep(for: .milliseconds(100))
            }
        case .connecting, .listening, .capturing, .thinking, .speaking, .cancelling:
            self.disconnect()
        }
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
            Task { try? await self.activeStt.reset() }
        }
        Task { await self.sendClient(.control(action: shouldMute ? .mute : .unmute)) }
    }

    @discardableResult
    public func interrupt() -> Bool {
        guard let task = self.task else { return false }
        switch self.state {
        case .thinking, .speaking, .capturing:
            break
        default:
            return false
        }
        let abort = ClientVoiceMessage.control(action: .abort)
        self.cancelClientTtsTurn()
        guard let data = try? Self.jsonEncoder.encode(abort),
              let text = String(data: data, encoding: .utf8)
        else { return false }
        Task { [weak self, weak task] in
            try? await task?.send(.string(text))
            await MainActor.run {
                self?.state = .cancelling
            }
        }
        return true
    }
}
