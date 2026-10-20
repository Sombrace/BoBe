import Foundation
import OSLog

nonisolated private let logger = Logger(subsystem: "com.bobe.app", category: "VoicePipeline.WebSocket")

extension VoicePipeline {

    func startReceiveLoop(socket: URLSessionWebSocketTask, generation: UInt64) {
        self.receiveTask?.cancel()
        self.receiveTask = Task { [weak self, weak socket] in
            guard let self, let socket else { return }
            do {
                while !Task.isCancelled {
                    let message = try await socket.receive()
                    guard !Task.isCancelled,
                          self.task === socket,
                          self.connectionGeneration == generation
                    else { return }
                    await self.handleIncoming(message)
                }
            } catch is CancellationError {
                return
            } catch {
                guard self.task === socket,
                      self.connectionGeneration == generation
                else { return }
                let reason = error.localizedDescription
                self.lastError = "recv: \(reason)"
                logger.error("recv: \(reason, privacy: .public)")
                self.tearDownConnection(finalState: .failed(reason), sendAbort: false)
            }
        }
    }

    private func handleIncoming(_ msg: URLSessionWebSocketTask.Message) async {
        switch msg {
        case let .string(text):
            await self.handleControlJson(text)
        case let .data(bytes):
            self.handleAudioFrame(bytes)
        @unknown default:
            break
        }
    }

    private func handleControlJson(_ text: String) async {
        guard let data = text.data(using: .utf8) else { return }
        let decoded: ServerVoiceMessage
        do {
            decoded = try Self.jsonDecoder.decode(ServerVoiceMessage.self, from: data)
        } catch {
            let errorType = String(describing: type(of: error))
            logger.warning(
                "malformed voice control frame bytes=\(data.count, privacy: .public) errorType=\(errorType, privacy: .public)"
            )
            return
        }
        switch decoded {
        case let .speechPlan(plan):
            guard self.validateSpeechPlan(plan) else {
                let reason = "Daemon selected an unavailable speech capability"
                self.lastError = reason
                self.tearDownConnection(finalState: .failed(reason), sendAbort: false)
                return
            }
            self.sessionSpeechPlan = plan
            self.handshakeComplete = true
            self.renegotiating = false
            self.sessionRestartError = nil
            logger.info(
                "voice.speech_plan stt=\(plan.sttSite.rawValue, privacy: .public) tts=\(plan.ttsSite.rawValue, privacy: .public)"
            )
        case let .captureCredit(streamId, creditFrames):
            guard creditFrames > 0 else {
                self.tearDownConnection(
                    finalState: .failed("Daemon sent invalid capture credit"),
                    sendAbort: false
                )
                return
            }
            if let active = self.captureStreamId, active != streamId {
                self.tearDownConnection(
                    finalState: .failed("Daemon changed capture stream identity"),
                    sendAbort: false
                )
                return
            }
            self.captureStreamId = streamId
            self.captureCreditFrames = self.captureCreditFrames.addingReportingOverflow(creditFrames).partialValue
        case let .sttPartial(_, text):
            self.partialTranscript = text
        case let .sttFinal(_, text):
            self.partialTranscript = text
        case let .state(phase, turnId):
            self.applyPhase(phase, turnId: turnId)
        case let .transcriptFinal(_, transcript):
            self.partialTranscript = ""
            if !transcript.isEmpty {
                BobeStore.shared.appendUserVoiceMessage(transcript)
            }
        case let .ttsEnd(turnId):
            self.partialTranscript = ""
            if self.sessionSpeechPlan?.ttsSite == .endpoint {
                await self.finishClientTtsTurn(turnId: turnId)
            } else {
                self.reportServerPlaybackDrained(turnId: turnId)
            }
        case let .ttsText(turnId, sequence, text):
            await self.enqueueClientTts(turnId: turnId, sequence: sequence, text: text)
        case let .truncate(turnId, _):
            if let playback = self.replyPlayback, playback.turnId != turnId {
                logger.debug("voice.stale_truncate_ignored")
                return
            }
            self.stopReplyPlayback()
        case let .error(code, message):
            let reason = "\(code): \(message)"
            self.terminalServerErrorCode = code
            self.lastError = reason
            logger.error("server: \(code, privacy: .public): \(message, privacy: .public)")
            self.tearDownConnection(finalState: .failed(reason), sendAbort: false)
        case .unknown:
            break
        }
    }

    private func validateSpeechPlan(_ plan: SpeechPlanWire) -> Bool {
        guard !plan.planId.isEmpty,
              plan.wakeOwner == .endpoint,
              let offer = self.sessionSpeechOffer
        else { return false }
        if plan.sttSite == .endpoint {
            let offered = offer.endpointStt.contains { engine in
                engine.ready && engine.id == plan.sttEngineId
            }
            guard offered,
                  self.sttLoaded,
                  plan.sttEngineId == self.effectiveSttEngineId,
                  plan.uplink == .transcript,
                  plan.eouOwner == .endpoint
            else { return false }
        } else {
            guard offer.audioUplink,
                  plan.uplink == .audio,
                  plan.eouOwner == .daemon
            else { return false }
        }
        if plan.ttsSite == .endpoint {
            let offered = offer.endpointTts.contains { engine in
                engine.ready
                    && engine.id == plan.ttsEngineId
                    && engine.voices.contains(plan.voiceId)
            }
            return offered
                && self.sessionTtsBackend == .clientSupertonic
                && plan.ttsEngineId == VoiceTtsBackend.clientSupertonic.rawValue
                && plan.downlink == .text
        }
        return plan.ttsEngineId == VoiceTtsBackend.serverPocket.rawValue
            && plan.downlink == .audio
    }

    private func applyPhase(_ phase: VoicePhaseWire, turnId: String) {
        guard let next = self.phaseTracker.state(after: phase, turnId: turnId, sessionId: self.sessionId) else {
            logger.info("voice.session_closed_by_daemon")
            self.tearDownConnection(finalState: .idle, sendAbort: false)
            return
        }
        switch next {
        case .listening:
            if self.clientTtsActive {
                self.pendingServerListening = true
                return
            }
            if self.state != .listening {
                self.state = .listening
            }
            self.stopTtsLevelDecay()
            self.drainPendingAudioDeviceRebuild()
        case .thinking:
            if self.state != .thinking {
                self.state = .thinking
            }
            self.stopTtsLevelDecay()
        case .speaking:
            if self.state != .speaking {
                self.scheduledChunks.removeAll(keepingCapacity: true)
                self.nextScheduleFrame = 0
                self.turnFrameBase = 0
                self.lastPlaybackAckMs = 0
                self.lastPlaybackAckChunkId = 0
                self.serverDrainTask?.cancel()
                self.serverDrainTask = nil
                self.resetReplyPosition()
                self.sampleTimeBase = self.currentPlayerSampleTime()
                self.startTtsLevelDecay()
                self.state = .speaking
            }
            if self.replyPlayback?.turnId != turnId {
                self.replyPlayback = ReplyPlayback(turnId: turnId)
            }
            self.ensureDecoder()
        case .idle, .connecting, .capturing, .cancelling, .failed:
            break
        }
    }

    func startKeepalive() {
        self.keepaliveTask?.cancel()
        self.keepaliveTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .nanoseconds(self?.keepalivePingIntervalNs ?? 25_000_000_000))
                guard !Task.isCancelled else { return }
                guard let self, let task = self.task else { return }
                task.sendPing { error in
                    if let error {
                        logger.warning(
                            "voice keepalive ping failed: \(error.localizedDescription, privacy: .public)"
                        )
                    }
                }
            }
        }
    }

    func stopKeepalive() {
        self.keepaliveTask?.cancel()
        self.keepaliveTask = nil
    }

    func sendClient(_ msg: ClientVoiceMessage) async {
        guard let task = self.task else { return }
        guard let data = try? Self.jsonEncoder.encode(msg),
              let text = String(data: data, encoding: .utf8)
        else { return }
        do {
            try await task.send(.string(text))
        } catch {
            let reason = error.localizedDescription
            self.lastError = "ws send text: \(reason)"
            logger.error("ws send text: \(reason, privacy: .public)")
            guard self.task === task else { return }
            self.tearDownConnection(finalState: .failed(reason), sendAbort: false)
        }
    }
}
