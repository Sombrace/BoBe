@preconcurrency import AVFoundation
import Foundation
import OSLog

nonisolated private let logger = Logger(subsystem: "com.bobe.app", category: "VoicePipeline.Audio")

extension VoicePipeline {

    func tearDownAudio() {
        self.audioLifecycle.invalidate()
        if self.inputTapInstalled {
            self.audioEngine.inputNode.removeTap(onBus: 0)
            self.inputTapInstalled = false
        }
        if self.playerTapInstalled {
            self.playerNode.removeTap(onBus: 0)
            self.playerTapInstalled = false
        }
        self.playerNode.stop()
        self.cancelClientTtsTurn()
        self.audioEngine.stop()
        if self.inputVoiceProcessingEnabled {
            try? self.audioEngine.inputNode.setVoiceProcessingEnabled(false)
            self.inputVoiceProcessingEnabled = false
        }
        if self.outputVoiceProcessingEnabled {
            try? self.audioEngine.outputNode.setVoiceProcessingEnabled(false)
            self.outputVoiceProcessingEnabled = false
        }
        self.audioInputContinuation?.finish()
        self.audioInputContinuation = nil
        let inputTask = self.audioInputTask
        self.audioInputTask = nil
        inputTask?.cancel()
        let processor = self.inputProcessor
        let engine = self.activeStt
        self.audioLifecycle.enqueue {
            await inputTask?.value
            await processor.reset()
            try? await engine.reset()
        }
        self.pendingTurnId = nil
        self.partialSendTask?.cancel()
        self.partialSendTask = nil
        self.pendingPartialForServer = nil
        self.bargeInCount = 0
        self.bargeInSent = false
        self.isWarm = false
    }

    func configureEngine() async throws {
        let generation = self.audioLifecycle.generation
        await self.audioLifecycle.settled()
        guard generation == self.audioLifecycle.generation else {
            throw CancellationError()
        }
        do {
            try self.configureEngineTransaction()
        } catch {
            self.tearDownAudio()
            throw error
        }
    }

    private func configureEngineTransaction() throws {
        let inputNode = self.audioEngine.inputNode
        try inputNode.setVoiceProcessingEnabled(true)
        self.inputVoiceProcessingEnabled = true
        try self.audioEngine.outputNode.setVoiceProcessingEnabled(true)
        self.outputVoiceProcessingEnabled = true

        let inputFormat = inputNode.inputFormat(forBus: 0)
        guard inputFormat.sampleRate > 0 else {
            throw VoiceError.runtime("input sample rate is 0 — no mic device?")
        }

        let (audioStream, continuation) = AsyncStream.makeStream(
            of: CapturedVoiceAudio.self,
            bufferingPolicy: .bufferingNewest(16)
        )
        self.audioInputContinuation = continuation
        let processor = self.inputProcessor
        self.audioInputTask = Task(name: "Voice audio input") { [weak self] in
            for await captured in audioStream {
                guard !Task.isCancelled else { return }
                do {
                    let processed = try await processor.process(captured)
                    guard let self, !Task.isCancelled else { return }
                    await self.handleProcessedInput(processed)
                } catch {
                    guard let self else { return }
                    self.lastError = "input processing: \(error.localizedDescription)"
                    logger.warning(
                        "input processing failed: \(error.localizedDescription, privacy: .public)"
                    )
                }
            }
        }

        let tapBlock: @Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void = { [weak self] buffer, _ in
            guard let captured = CapturedVoiceAudio(buffer: buffer) else { return }
            if case .dropped = continuation.yield(captured) {
                DispatchQueue.main.async {
                    guard let self else { return }
                    self.droppedAudioInputChunks += 1
                    logger.warning(
                        "voice.input_queue_drop total=\(self.droppedAudioInputChunks)"
                    )
                    if self.sessionSpeechPlan?.sttSite == .daemon {
                        let reason = "Microphone audio could not be streamed continuously"
                        self.lastError = reason
                        self.tearDownConnection(finalState: .failed(reason), sendAbort: false)
                    }
                }
            }
        }
        inputNode.installTap(onBus: 0, bufferSize: 2048, format: inputFormat, block: tapBlock)
        self.inputTapInstalled = true

        let playerFormat = self.playerNode.outputFormat(forBus: 0)
        if playerFormat.sampleRate > 0 {
            let playerTapBlock: @Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void = { [weak self] buffer, _ in
                let rms = computePlaybackRms(buffer)
                guard rms.isFinite else { return }
                DispatchQueue.main.async {
                    self?.handlePlaybackRms(rms)
                }
            }
            self.playerNode.installTap(
                onBus: 0,
                bufferSize: 1_024,
                format: playerFormat,
                block: playerTapBlock
            )
            self.playerTapInstalled = true
        }

        self.audioEngine.prepare()
        try self.audioEngine.start()
        self.playerNode.play()
        self.registerInputDeviceObserverIfNeeded()
        logger.info("voice engine configured; input \(describe(inputFormat), privacy: .public)")
    }

    func handleProcessedInput(_ processed: ProcessedVoiceAudio) async {
        guard self.task != nil, self.state.acceptsMicrophoneInput else { return }

        guard !self.muted else {
            self.inputLevel = 0
            return
        }

        for frameRms in processed.rmsFramesDbfs {
            self.publishInputLevel(frameRms: frameRms)
            self.checkBargeIn(frameRms: frameRms)
        }

        if self.sessionSpeechPlan?.sttSite == .daemon {
            await self.sendCaptureSamples(processed.samples16k)
            return
        }
        guard self.sttStatus == .ready, self.sttLoaded else { return }
        do {
            try await self.activeStt.acceptSamples(processed.samples16k)
        } catch {
            if case .notLoaded = error {
                logger.debug("stt.acceptSamples_notLoaded_race")
            } else {
                logger.warning(
                    "stt.acceptSamples failed: \(error.localizedDescription, privacy: .public)"
                )
            }
        }
    }

    private func sendCaptureSamples(_ samples: [Float]) async {
        self.captureSamples.append(contentsOf: samples)
        while self.captureSamples.count >= VoiceWire.captureFrameSamples {
            guard let streamId = self.captureStreamId,
                  self.captureCreditFrames > 0,
                  let task = self.task
            else {
                if self.captureSamples.count > VoiceWire.captureSampleRate {
                    let reason = "Speech server stopped accepting microphone audio"
                    self.lastError = reason
                    self.tearDownConnection(finalState: .failed(reason), sendAbort: false)
                }
                return
            }
            guard let frameSamples = self.captureSamples.firstFrame(count: VoiceWire.captureFrameSamples),
                  let frame = VoiceCaptureFrameEncoder.encode(
                      streamId: streamId,
                      sequence: self.captureSequence,
                      samples: frameSamples
                  )
            else { return }
            do {
                try await task.send(.data(frame))
            } catch {
                let reason = "microphone uplink: \(error.localizedDescription)"
                self.lastError = reason
                self.tearDownConnection(finalState: .failed(reason), sendAbort: false)
                return
            }
            guard self.task === task,
                  self.captureStreamId == streamId,
                  self.captureCreditFrames > 0,
                  self.captureSamples.consumeFirst(VoiceWire.captureFrameSamples)
            else { return }
            self.captureSequence &+= 1
            self.captureCreditFrames -= 1
        }
    }

    private func publishInputLevel(frameRms: Float) {
        let floor: Float = -50
        let normalized = max(0, min(1, (frameRms - floor) / -floor))
        let smoothed = (self.inputLevel * 0.6) + (normalized * 0.4)
        if abs(smoothed - self.inputLevel) > 0.005 {
            self.inputLevel = smoothed
        }
    }

    private func checkBargeIn(frameRms: Float) {
        guard self.state == .speaking else {
            self.bargeInCount = 0
            self.bargeInSent = false
            self.setPlaybackDucked(false)
            return
        }
        let playedMs = self.currentPlaybackMs()
        if let audibleMs = Playhead.audibleMilliseconds(
            played: self.playedFramesThisTurn(),
            audioStart: self.turnAudioStartFrame,
            sampleRate: self.playbackSampleRate
        ), audibleMs < BargeInPolicy.echoGuardMilliseconds {
            self.bargeInCount = 0
            return
        }
        guard frameRms > self.bargeInRmsDbfs else {
            self.bargeInCount = 0
            if self.playbackDucked,
               self.lastOverlapAt.duration(to: .now) > BargeInPolicy.unduckAfter {
                self.setPlaybackDucked(false)
            }
            return
        }
        self.bargeInCount += 1
        guard self.bargeInCount >= self.bargeInFramesNeeded else { return }
        self.lastOverlapAt = .now
        self.setPlaybackDucked(true)
        guard !self.bargeInSent else { return }
        self.bargeInSent = true
        let playheadChunk = self.overlapPlayheadChunk ?? self.playheadChunk()
        let tsMs = UInt64(Date.now.timeIntervalSince1970 * 1_000)
        logger.info("voice.barge_in_sending playedMs=\(playedMs)")
        let evidence = self.partialTranscript
        let turnId = self.replyPlayback?.turnId
        Task.immediate { [weak self] in
            guard let self else { return }
            await self.sendClient(.bargeIn(
                tsMs: tsMs,
                playbackMsPlayed: playedMs,
                partialText: evidence.isEmpty ? nil : evidence,
                playheadChunk: playheadChunk,
                turnId: turnId
            ))
            try? await Task.sleep(for: .milliseconds(250))
            guard self.state == .speaking else { return }
            self.bargeInSent = false
            self.bargeInCount = 0
        }
    }

    func setPlaybackDucked(_ ducked: Bool) {
        guard ducked != self.playbackDucked else { return }
        self.playbackDucked = ducked
        self.overlapPlayheadChunk = ducked ? self.playheadChunk() : nil
        self.duckTask?.cancel()
        let target: Float = ducked ? BargeInPolicy.duckedVolume : 1
        let start = self.playerNode.volume
        self.duckTask = Task { [weak self] in
            for step in 1 ... 5 {
                guard let self, !Task.isCancelled else { return }
                self.playerNode.volume = start + (target - start) * Float(step) / 5
                try? await Task.sleep(for: .milliseconds(12))
            }
        }
    }

    private func currentPlaybackMs() -> UInt64 {
        let frames = self.playedFramesThisTurn()
        let ms = Double(frames) * 1_000.0 / self.playbackSampleRate
        return UInt64(max(0, ms))
    }
}
