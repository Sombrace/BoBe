import Foundation
import OSLog

nonisolated private let logger = Logger(subsystem: "com.bobe.app", category: "VoicePipeline.Session")

struct VoicePhaseTracker {
    private(set) var unacknowledgedAborts = 0

    mutating func abortRequested() {
        self.unacknowledgedAborts += 1
    }

    mutating func state(
        after phase: VoicePhaseWire,
        turnId: String,
        sessionId: String
    ) -> VoicePipeline.State? {
        switch phase {
        case .idle:
            guard self.unacknowledgedAborts > 0 else { return nil }
            self.unacknowledgedAborts -= 1
            return .listening
        case .listening:
            if turnId == sessionId, self.unacknowledgedAborts > 0 {
                self.unacknowledgedAborts -= 1
            }
            return .listening
        case .thinking:
            return .thinking
        case .speaking:
            return .speaking
        }
    }
}

extension VoicePipeline.State {
    var hasSession: Bool {
        switch self {
        case .idle, .failed: false
        case .connecting, .listening, .capturing, .thinking, .speaking, .cancelling: true
        }
    }

    var acceptsMicrophoneInput: Bool {
        switch self {
        case .idle, .connecting, .failed: false
        case .listening, .capturing, .thinking, .speaking, .cancelling: true
        }
    }
}

extension VoicePipeline {
    static func sttLanguage(from settings: DaemonSettings?) -> String {
        let language = settings?.voiceSttLanguage ?? ""
        return language.isEmpty ? "en" : language
    }

    public func connect(daemonBaseURL: URL) {
        guard self.task == nil else {
            logger.debug("voice.connect_ignored_session_open")
            return
        }
        self.lastError = nil
        self.terminalServerErrorCode = nil
        self.sessionRestartError = nil

        guard let wsURL = voiceWsEndpoint(from: daemonBaseURL) else {
            self.lastError = "invalid daemon URL for WS"
            logger.error("invalid daemon URL for WS")
            return
        }

        let settings = VoiceSessionSettings.current()
        self.applySessionSettings(settings)
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
        self.handshakeTask = Task { [weak self, weak newTask] in
            guard let self, let newTask else { return }
            await self.negotiate(socket: newTask, generation: generation, settings: settings)
        }
    }

    private func applySessionSettings(_ settings: VoiceSessionSettings) {
        self.sessionSttLanguage = settings.language
        self.sessionTtsBackend = settings.ttsBackend
        self.sessionVoicePersona = settings.persona
        self.sessionVoiceSpeed = settings.speed
    }

    private func isCurrent(_ socket: URLSessionWebSocketTask, _ generation: UInt64) -> Bool {
        !Task.isCancelled && self.task === socket && self.connectionGeneration == generation
    }

    private func negotiate(
        socket: URLSessionWebSocketTask,
        generation: UInt64,
        settings: VoiceSessionSettings
    ) async {
        guard self.isCurrent(socket, generation) else { return }
        let warmed = await self.prewarm()
        guard self.isCurrent(socket, generation) else { return }
        guard warmed else {
            let reason = self.lastError ?? "voice audio setup failed"
            self.tearDownConnection(finalState: .failed(reason), sendAbort: false)
            return
        }
        let speechOffer = self.makeSpeechOffer(settings: settings)
        self.sessionSpeechOffer = speechOffer
        self.negotiatedSettings = settings
        await self.sendClient(.hello(
            sessionId: self.sessionId,
            voiceId: settings.persona,
            speed: settings.speed,
            speechOffer: speechOffer
        ))
        if self.muted {
            await self.sendClient(.control(action: .mute))
        }
        guard self.isCurrent(socket, generation) else { return }
        self.reconcileSessionSettings(VoiceSessionSettings.current())
        let negotiationAttempts = DaemonConfig.endpoint.isLoopbackRuntime ? 20 : 80
        for _ in 0 ..< negotiationAttempts {
            if self.handshakeComplete || self.state != .connecting { return }
            try? await Task.sleep(for: .milliseconds(100))
            if Task.isCancelled { return }
        }
        guard self.isCurrent(socket, generation), self.state == .connecting else { return }
        let reason = self.lastError ?? "voice negotiation timed out"
        self.lastError = reason
        self.tearDownConnection(finalState: .failed(reason), sendAbort: false)
    }

    func reconcileSessionSettings(_ desired: VoiceSessionSettings) {
        guard self.task != nil, let negotiated = self.negotiatedSettings else { return }
        let adopted = negotiated.adoptingDaemonDefaults(from: desired)
        self.negotiatedSettings = adopted
        guard adopted != desired else { return }
        self.renegotiationTask?.cancel()
        self.renegotiationTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(500))
            guard let self, !Task.isCancelled else { return }
            while self.state == .connecting, self.task != nil, !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(100))
            }
            guard !Task.isCancelled else { return }
            self.renegotiationTask = nil
            await self.renegotiateSession()
        }
    }

    private func renegotiateSession() async {
        guard let socket = self.task, let negotiated = self.negotiatedSettings else { return }
        let desired = VoiceSessionSettings.current()
        guard negotiated.adoptingDaemonDefaults(from: desired) != desired else { return }
        logger.info("voice.session_settings_changed renegotiating")
        let previousEngine = self.activeStt
        self.stopReplyPlayback()
        self.replyPlayback = nil
        self.resetUtterance()
        self.resetCaptureStream()
        self.sessionSpeechPlan = nil
        self.handshakeComplete = false
        self.applySessionSettings(desired)
        self.renegotiating = true
        self.state = .connecting
        try? await previousEngine.reset()
        await self.negotiate(socket: socket, generation: self.connectionGeneration, settings: desired)
    }

    private func resetUtterance() {
        self.inputLevel = 0
        self.partialTranscript = ""
        self.pendingTurnId = nil
        self.clearHeldUtterance()
        self.pendingPartialForServer = nil
        self.partialSendTask?.cancel()
        self.partialSendTask = nil
    }

    private func resetCaptureStream() {
        self.captureStreamId = nil
        self.captureSequence = 0
        self.captureCreditFrames = 0
        self.captureSamples.removeAll()
    }

    private func makeSpeechOffer(settings: VoiceSessionSettings) -> SpeechOfferWire {
        Self.speechOffer(
            settings: settings,
            recognizerReady: self.sttLoaded,
            systemVoice: settings.ttsBackend == .systemVoice
                ? SystemVoiceChoice.best(for: settings.language)?.identifier
                : nil,
            endpointId: SurfaceIdentity.current(),
            daemonIsNonLoopback: !DaemonConfig.endpoint.isLoopbackRuntime
        )
    }

    static func speechOffer(
        settings: VoiceSessionSettings,
        recognizerReady: Bool,
        systemVoice: String?,
        endpointId: UUID,
        daemonIsNonLoopback: Bool
    ) -> SpeechOfferWire {
        let language = settings.language
        let endpointStt = recognizerReady ? [SpeechEngineOfferWire(
            id: VoiceEngineId.appleSpeechAnalyzer,
            languages: [language],
            partials: true,
            eou: true,
            voices: [],
            sampleRates: [],
            ready: true
        )] : []
        let endpointTts = systemVoice.map { voice in
            [SpeechEngineOfferWire(
                id: VoiceEngineId.appleSpeechSynthesizer,
                languages: [language],
                partials: false,
                eou: false,
                voices: [voice],
                sampleRates: [UInt32(VoiceWire.ttsOutputSampleRate)],
                ready: true
            )]
        } ?? []
        return SpeechOfferWire(
            endpointId: endpointId,
            endpointKind: SpeechOfferWire.macEndpointKind,
            language: language,
            playbackRates: [UInt32(VoiceWire.ttsOutputSampleRate)],
            endpointStt: endpointStt,
            endpointTts: endpointTts,
            audioUplink: VoiceTtsBackend.languageKey(language) == "en",
            daemonIsNonLoopback: daemonIsNonLoopback,
            remoteAudioConsent: settings.remoteAudioConsent
        )
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
        self.renegotiationTask?.cancel()
        self.renegotiationTask = nil
        if self.renegotiating, case let .failed(reason) = finalState {
            self.sessionRestartError = reason
        }
        self.renegotiating = false
        self.phaseTracker = VoicePhaseTracker()
        self.connectionGeneration &+= 1
        let oldTask = self.task
        self.task = nil
        self.state = finalState
        self.resetUtterance()
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
        self.sessionTtsBackend = nil
        self.sessionVoicePersona = nil
        self.sessionVoiceSpeed = nil
        self.sessionSpeechPlan = nil
        self.sessionSpeechOffer = nil
        self.negotiatedSettings = nil
        self.handshakeComplete = false
        self.resetCaptureStream()
        self.playbackAckTask?.cancel()
        self.playbackAckTask = nil
        self.playbackAckGeneration &+= 1
        self.lastPlaybackAckMs = 0
        self.lastPlaybackAckChunkId = 0
        self.serverDrainTask?.cancel()
        self.serverDrainTask = nil
        self.replyPlayback = nil
        self.resetReplyPosition()
    }

    public func toggle(daemonBaseURL: URL) async {
        guard !self.state.hasSession else {
            self.disconnect()
            return
        }
        self.connect(daemonBaseURL: daemonBaseURL)
        while self.state == .connecting, self.task != nil {
            try? await Task.sleep(for: .milliseconds(100))
        }
    }

    @discardableResult
    public func interrupt() -> Bool {
        guard self.task != nil else { return false }
        switch self.state {
        case .thinking, .speaking, .capturing:
            break
        default:
            return false
        }
        self.stopReplyPlayback()
        self.abortCurrentTurn()
        return true
    }

    func abortCurrentTurn() {
        self.state = .cancelling
        self.phaseTracker.abortRequested()
        Task { await self.sendClient(.control(action: .abort)) }
    }
}
