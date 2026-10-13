@preconcurrency import AVFoundation
import Foundation
import OSLog

private let logger = Logger(subsystem: "com.bobe.app", category: "TtsPlayback")

struct ScheduledTtsChunk {
    let chunkId: UInt64
    let buffer: AVAudioPCMBuffer
    let startFrame: AVAudioFramePosition
    var isFiller = false

    var endFrame: AVAudioFramePosition {
        self.startFrame + AVAudioFramePosition(self.buffer.frameLength)
    }
}

@MainActor
extension VoicePipeline {

    func ensureDecoder() {
        if self.decoder != nil { return }
        guard let format = self.ttsOutputFormat else { return }
        do {
            self.decoder = try OpusDecoder(outputFormat: format)
        } catch {
            self.lastError = "decoder init: \(error.localizedDescription)"
            logger.error("decoder init: \(error.localizedDescription, privacy: .public)")
        }
    }

    func handleAudioFrame(_ bytes: Data) {
        guard let parsed = TtsFrameHeader.parse(bytes) else {
            logger.warning("voice.malformed_tts_frame size=\(bytes.count)")
            return
        }
        self.ensureDecoder()
        guard let decoder = self.decoder, let format = self.ttsOutputFormat else { return }
        guard let outBuffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 1_440)
        else { return }
        do {
            try parsed.payload.withUnsafeBytes { raw in
                let typed = UnsafeBufferPointer(
                    start: raw.bindMemory(to: UInt8.self).baseAddress,
                    count: raw.count
                )
                try decoder.decode(typed, to: outBuffer)
            }
            let chunk = ScheduledTtsChunk(
                chunkId: parsed.header.chunkId,
                buffer: outBuffer,
                startFrame: Playhead.startFrame(
                    scheduledThrough: self.nextScheduleFrame,
                    played: self.playedFramesThisTurn()
                ),
                isFiller: parsed.header.isFiller
            )
            self.noteScheduled(chunk)
            self.playerNode.scheduleBuffer(outBuffer, completionHandler: nil)
            self.schedulePlaybackAcknowledgement(chunkId: parsed.header.chunkId)
            self.trimPlayedChunks()
        } catch {
            self.lastError = "decode: \(error.localizedDescription)"
            logger.error("decode: \(error.localizedDescription, privacy: .public)")
        }
    }

    func noteScheduled(_ chunk: ScheduledTtsChunk) {
        self.scheduledChunks.append(chunk)
        self.nextScheduleFrame = chunk.endFrame
        if self.turnAudioStartFrame == nil {
            self.turnAudioStartFrame = chunk.startFrame
        }
        if !chunk.isFiller {
            self.nextReplyChunkId = chunk.chunkId &+ 1
        }
    }

    func resetReplyPosition() {
        self.turnAudioStartFrame = nil
        self.nextReplyChunkId = 0
        self.overlapPlayheadChunk = nil
    }

    func playheadChunk() -> UInt64 {
        Playhead.chunk(
            replySpans: self.scheduledChunks.lazy
                .filter { !$0.isFiller }
                .map { PlayheadSpan(chunkId: $0.chunkId, endFrame: $0.endFrame) },
            played: self.playedFramesThisTurn(),
            nextReplyChunk: self.nextReplyChunkId
        )
    }

    func waitForPlaybackDrain() async {
        let remainingFrames = max(0, self.nextScheduleFrame - self.playedFramesThisTurn())
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(
            by: .seconds((Double(remainingFrames) / self.playbackSampleRate) + 5)
        )
        while self.audioStillPlaying(), clock.now < deadline, !Task.isCancelled {
            try? await clock.sleep(for: .milliseconds(40))
        }
    }

    func reportServerPlaybackDrained(turnId: String) {
        self.serverDrainTask?.cancel()
        self.serverDrainTask = Task { @MainActor [weak self] in
            guard let self else { return }
            await self.waitForPlaybackDrain()
            guard !Task.isCancelled else { return }
            self.serverDrainTask = nil
            await self.sendPlaybackAcknowledgement()
            await self.sendClient(.ttsPlaybackComplete(turnId: turnId))
        }
    }

    func schedulePlaybackAcknowledgement(chunkId: UInt64) {
        self.lastPlaybackAckChunkId = max(self.lastPlaybackAckChunkId, chunkId)
        guard self.playbackAckTask == nil else { return }
        self.playbackAckGeneration &+= 1
        let generation = self.playbackAckGeneration
        self.playbackAckTask = Task { @MainActor [weak self] in
            defer {
                if let self, self.playbackAckGeneration == generation {
                    self.playbackAckTask = nil
                }
            }
            while !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(250))
                guard let self, !Task.isCancelled else { return }
                await self.sendPlaybackAcknowledgement()
                if !self.audioStillPlaying() { return }
            }
        }
    }

    func sendPlaybackAcknowledgement() async {
        let frames = max(0, self.playedFramesThisTurn())
        let playedMs = UInt64(Double(frames) * 1_000.0 / self.playbackSampleRate)
        guard playedMs > self.lastPlaybackAckMs else { return }
        self.lastPlaybackAckMs = playedMs
        await self.sendClient(.playbackAck(
            chunkId: self.lastPlaybackAckChunkId,
            playedMs: playedMs
        ))
    }

    func trimPlayedChunks() {
        let played = self.playedFramesThisTurn()
        let retainTailFrames = AVAudioFramePosition(self.playbackSampleRate * 2.0)
        let cutoff = played - retainTailFrames
        guard cutoff > 0 else { return }
        var dropCount = 0
        for chunk in self.scheduledChunks {
            let chunkEnd = chunk.startFrame + AVAudioFramePosition(chunk.buffer.frameLength)
            if chunkEnd <= cutoff {
                dropCount += 1
            } else {
                break
            }
        }
        if dropCount > 0 {
            self.scheduledChunks.removeFirst(dropCount)
        }
    }


    func playedFramesThisTurn() -> AVAudioFramePosition {
        guard let lrt = self.playerNode.lastRenderTime,
              let pt = self.playerNode.playerTime(forNodeTime: lrt)
        else {
            return self.turnFrameBase
        }
        let st = max(self.sampleTimeBase, pt.sampleTime)
        return self.turnFrameBase + (st - self.sampleTimeBase)
    }

    func currentPlayerSampleTime() -> AVAudioFramePosition {
        guard let lrt = self.playerNode.lastRenderTime,
              let pt = self.playerNode.playerTime(forNodeTime: lrt)
        else {
            return 0
        }
        return max(0, pt.sampleTime)
    }


    func truncatePlayback() {
        self.serverDrainTask?.cancel()
        self.serverDrainTask = nil
        self.playbackAckTask?.cancel()
        self.playbackAckTask = nil
        self.playbackAckGeneration &+= 1
        Task { [weak self] in
            await self?.sendPlaybackAcknowledgement()
        }
        let played = self.playedFramesThisTurn()

        self.playerNode.stop()
        self.bargeInCount = 0
        self.bargeInSent = false
        self.setPlaybackDucked(false)
        self.scheduledChunks.removeAll(keepingCapacity: true)
        self.nextScheduleFrame = played
        self.turnFrameBase = played
        self.sampleTimeBase = self.currentPlayerSampleTime()
        self.playerNode.play()
    }


    func handlePlaybackRms(_ rms: Float) {
        let normalized = Self.normalizeRmsToLevel(rms)
        let next = normalized > self.ttsOutputLevel
            ? (self.ttsOutputLevel * 0.35) + (normalized * 0.65)
            : (self.ttsOutputLevel * 0.7) + (normalized * 0.3)
        if abs(next - self.ttsOutputLevel) > 0.005 {
            self.ttsOutputLevel = next
        }
        self.updateAudibilityLatch(level: next)
        self.lastTtsRmsAt = ContinuousClock.now
    }

    private func updateAudibilityLatch(level: Float) {
        if level >= self.audibleOnThreshold {
            self.audibleHangoverTask?.cancel()
            self.audibleHangoverTask = nil
            if !self.isTtsAudible {
                self.isTtsAudible = true
            }
            return
        }
        if level < self.audibleOffThreshold, self.isTtsAudible, self.audibleHangoverTask == nil {
            self.audibleHangoverTask = Task { @MainActor [weak self] in
                guard let self else { return }
                try? await Task.sleep(for: .milliseconds(self.audibleHangoverMs))
                guard !Task.isCancelled else { return }
                if self.ttsOutputLevel < self.audibleOffThreshold {
                    self.isTtsAudible = false
                }
                self.audibleHangoverTask = nil
            }
        }
    }

    func startTtsLevelDecay() {
        self.ttsLevelDecayTask?.cancel()
        self.ttsLevelDecayTask = Task { @MainActor [weak self] in
            while let self, !Task.isCancelled {
                if self.shouldExitDecayLoop() {
                    self.ttsOutputLevel = 0
                    self.audibleHangoverTask?.cancel()
                    self.audibleHangoverTask = nil
                    if self.isTtsAudible { self.isTtsAudible = false }
                    return
                }
                if let lastTtsRmsAt = self.lastTtsRmsAt,
                   lastTtsRmsAt.duration(to: .now) > .milliseconds(60) {
                    self.ttsOutputLevel *= 0.95
                    if self.ttsOutputLevel < 0.01 { self.ttsOutputLevel = 0 }
                }
                try? await Task.sleep(for: .milliseconds(16))
            }
        }
    }

    func stopTtsLevelDecay() {
        if self.audioStillPlaying() { return }
        self.ttsLevelDecayTask?.cancel()
        self.ttsLevelDecayTask = nil
        self.ttsOutputLevel = 0
        self.audibleHangoverTask?.cancel()
        self.audibleHangoverTask = nil
        if self.isTtsAudible { self.isTtsAudible = false }
    }


    private static func normalizeRmsToLevel(_ rms: Float) -> Float {
        let dbfs = 20 * log10(max(rms, 1e-6))
        return max(0, min(1, (dbfs + 60) / 60))
    }

    func audioStillPlaying() -> Bool {
        self.playedFramesThisTurn() < self.nextScheduleFrame
    }

    private func shouldExitDecayLoop() -> Bool {
        self.state != .speaking
            && !self.audioStillPlaying()
            && self.ttsOutputLevel < 0.02
    }
}
