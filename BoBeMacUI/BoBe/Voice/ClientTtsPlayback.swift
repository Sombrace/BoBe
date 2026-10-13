@preconcurrency import AVFoundation
import Foundation
import OSLog

private let logger = Logger(subsystem: "com.bobe.app", category: "ClientTtsPlayback")

struct ClientTtsSentence: Sendable {
    let turnId: String
    let sequence: UInt64
    let text: String
}

actor ClientTtsSentenceQueue {
    private struct PendingSend {
        let sentence: ClientTtsSentence
        let continuation: CheckedContinuation<Bool, Never>
    }

    private let capacity: Int
    private var buffer: [ClientTtsSentence] = []
    private var pendingSends: [PendingSend] = []
    private var pendingReceive: CheckedContinuation<ClientTtsSentence?, Never>?
    private var closed = false

    init(capacity: Int = 16) {
        precondition(capacity > 0)
        self.capacity = capacity
        self.buffer.reserveCapacity(capacity)
    }

    func send(_ sentence: ClientTtsSentence) async -> Bool {
        guard !self.closed else { return false }
        if let receiver = self.pendingReceive {
            self.pendingReceive = nil
            receiver.resume(returning: sentence)
            return true
        }
        if self.buffer.count < self.capacity {
            self.buffer.append(sentence)
            return true
        }
        return await withCheckedContinuation { continuation in
            self.pendingSends.append(PendingSend(
                sentence: sentence,
                continuation: continuation
            ))
        }
    }

    func next() async -> ClientTtsSentence? {
        if !self.buffer.isEmpty {
            let sentence = self.buffer.removeFirst()
            self.admitPendingSend()
            return sentence
        }
        guard !self.closed else { return nil }
        return await withCheckedContinuation { continuation in
            precondition(self.pendingReceive == nil)
            self.pendingReceive = continuation
        }
    }

    func finish() {
        self.closed = true
        if self.buffer.isEmpty, let receiver = self.pendingReceive {
            self.pendingReceive = nil
            receiver.resume(returning: nil)
        }
        self.rejectPendingSends()
    }

    func cancel() {
        self.closed = true
        self.buffer.removeAll(keepingCapacity: true)
        if let receiver = self.pendingReceive {
            self.pendingReceive = nil
            receiver.resume(returning: nil)
        }
        self.rejectPendingSends()
    }

    private func admitPendingSend() {
        guard !self.closed, !self.pendingSends.isEmpty else { return }
        let pending = self.pendingSends.removeFirst()
        self.buffer.append(pending.sentence)
        pending.continuation.resume(returning: true)
    }

    private func rejectPendingSends() {
        let sends = self.pendingSends
        self.pendingSends.removeAll(keepingCapacity: true)
        for pending in sends {
            pending.continuation.resume(returning: false)
        }
    }
}

@MainActor
extension VoicePipeline {
    func enqueueClientTts(turnId: String, sequence: UInt64, text: String) async {
        if turnId == self.cancelledClientTtsTurnId {
            logger.debug("client TTS sentence for a cancelled turn ignored")
            return
        }
        if let activeTurnId = self.clientTtsTurnId, activeTurnId != turnId {
            logger.debug("stale client TTS sentence ignored")
            return
        }
        if self.clientTtsQueue == nil {
            self.startClientTtsTurn(turnId: turnId)
        }
        guard self.clientTtsTurnId == turnId,
              let queue = self.clientTtsQueue
        else { return }
        let accepted = await queue.send(
            ClientTtsSentence(turnId: turnId, sequence: sequence, text: text)
        )
        if !accepted {
            logger.debug("client TTS sentence rejected after turn closure")
        }
    }

    func finishClientTtsTurn(turnId: String) async {
        guard self.clientTtsTurnId == turnId,
              let queue = self.clientTtsQueue
        else {
            logger.debug("stale client TTS end ignored")
            return
        }
        self.clientTtsQueue = nil
        await queue.finish()
    }

    func cancelClientTtsTurn() {
        guard self.clientTtsActive else { return }
        self.cancelledClientTtsTurnId = self.clientTtsTurnId
        if let queue = self.clientTtsQueue {
            Task { await queue.cancel() }
        }
        self.clientTtsQueue = nil
        self.clientTtsTask?.cancel()
        self.clientTtsTask = nil
        self.clientTtsTurnId = nil
        self.clientTtsActive = false
        self.clientTtsFirstAudioSent = false
        self.pendingServerListening = false
        self.playerNode.stop()
        self.scheduledChunks.removeAll(keepingCapacity: true)
        self.nextScheduleFrame = 0
        self.turnFrameBase = 0
        self.lastPlaybackAckMs = 0
        self.lastPlaybackAckChunkId = 0
        self.resetReplyPosition()
        self.sampleTimeBase = self.currentPlayerSampleTime()
        self.playerNode.play()
    }

    private func startClientTtsTurn(turnId: String) {
        self.cancelClientTtsTurn()
        let queue = ClientTtsSentenceQueue()
        self.clientTtsQueue = queue
        self.clientTtsTurnId = turnId
        self.clientTtsActive = true
        self.clientTtsFirstAudioSent = false
        self.pendingServerListening = false
        let engine = self.clientTtsEngine
        let language = self.effectiveLanguage
        let voiceId = self.sessionVoicePersona ?? self.voicePersona
        let speed = self.sessionVoiceSpeed ?? self.voiceSpeed ?? 1

        self.clientTtsTask = Task { @MainActor [weak self] in
            guard let self else { return }
            while let sentence = await queue.next() {
                guard !Task.isCancelled, sentence.turnId == turnId else { return }
                do {
                    let synthesisStart = ContinuousClock.now
                    let samples = try await engine.synthesize(
                        text: sentence.text,
                        language: language,
                        voiceId: voiceId,
                        speed: speed
                    )
                    guard !Task.isCancelled else { return }
                    self.scheduleClientTts(samples, sequence: sentence.sequence)
                    if !self.clientTtsFirstAudioSent {
                        self.clientTtsFirstAudioSent = true
                        let synthesisMs = synthesisStart.duration(to: .now)
                            .components.seconds * 1_000
                            + synthesisStart.duration(to: .now).components.attoseconds
                                / 1_000_000_000_000_000
                        await self.sendClient(
                            .ttsPlaybackStarted(
                                turnId: turnId,
                                synthesisMs: UInt64(max(0, synthesisMs))
                            )
                        )
                    }
                } catch is CancellationError {
                    return
                } catch {
                    logger.error(
                        "client TTS failed: \(error.localizedDescription, privacy: .public)"
                    )
                }
            }
            await self.waitForPlaybackDrain()
            guard !Task.isCancelled, self.clientTtsTurnId == turnId else { return }
            await self.sendPlaybackAcknowledgement()
            await self.sendClient(.ttsPlaybackComplete(turnId: turnId))
            guard self.clientTtsTurnId == turnId else { return }
            self.clientTtsTurnId = nil
            self.clientTtsActive = false
            self.clientTtsTask = nil
            self.clientTtsFirstAudioSent = false
            if self.pendingServerListening {
                self.pendingServerListening = false
                self.state = .listening
                self.stopTtsLevelDecay()
                self.drainPendingAudioDeviceRebuild()
            }
        }
    }

    private func scheduleClientTts(_ samples: [Int16], sequence: UInt64) {
        guard let format = self.ttsOutputFormat,
              let buffer = AVAudioPCMBuffer(
                  pcmFormat: format,
                  frameCapacity: AVAudioFrameCount(samples.count)
              ),
              let data = buffer.int16ChannelData?[0]
        else { return }
        buffer.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { source in
            guard let base = source.baseAddress else { return }
            data.update(from: base, count: source.count)
        }
        self.noteScheduled(
            ScheduledTtsChunk(
                chunkId: sequence,
                buffer: buffer,
                startFrame: Playhead.startFrame(
                    scheduledThrough: self.nextScheduleFrame,
                    played: self.playedFramesThisTurn()
                )
            )
        )
        self.playerNode.scheduleBuffer(buffer, completionHandler: nil)
        self.schedulePlaybackAcknowledgement(chunkId: sequence)
        if !self.playerNode.isPlaying {
            self.playerNode.play()
        }
        self.trimPlayedChunks()
    }
}
