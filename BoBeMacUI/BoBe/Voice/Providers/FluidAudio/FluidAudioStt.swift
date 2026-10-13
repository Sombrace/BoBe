import AVFoundation
import FluidAudio
import Foundation
import OSLog

actor FluidAudioStt: VoiceSttEngine {
    private let logger = Logger(subsystem: "com.bobe.app", category: "FluidAudioStt")
    private let chunkSize: StreamingChunkSize
    private var eouDebounceMs: Int
    private var manager: StreamingEouAsrManager?
    private var loaded = false
    private var loadTask: Task<Void, Error>?
    private let commitLayer = PunctuationCommitLayer()
    private var savedOnPartial: (@Sendable (String) async -> Void)?
    private var savedOnEou: (@Sendable (String) async -> Void)?
    private var callbackContinuation: AsyncStream<VoiceSttCallbackEvent>.Continuation?
    private var callbackTask: Task<Void, Never>?
    private let inputFormat = AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: 16_000,
        channels: 1,
        interleaved: false
    )

    init(
        chunkSize: StreamingChunkSize = .ms320,
        eouDebounceMs: Int = PauseSensitivityMs.balanced
    ) {
        self.chunkSize = chunkSize
        self.eouDebounceMs = eouDebounceMs
    }

    func setEouDebounceMs(_ value: Int) async {
        self.eouDebounceMs = max(100, min(5_000, value))
        if let mgr = self.manager {
            await mgr.updateEouDebounceMs(self.eouDebounceMs)
        }
    }

    private func handleRawPartial(_ text: String) async {
        let update = await self.commitLayer.processPartialText(text)
        await self.savedOnPartial?(update.totalText)
    }

    private func handleRawEou(_ text: String) async {
        let update = await self.commitLayer.processEOU()
        let final = update.committedText.isEmpty ? text : update.committedText
        await self.manager?.reset()
        await self.commitLayer.reset()
        await self.savedOnEou?(final)
    }

    func loadModels(
        onPartial: @escaping @Sendable (String) async -> Void,
        onEou: @escaping @Sendable (String) async -> Void,
        onFailure _: @escaping @Sendable (String) async -> Void,
        onProgress: @escaping @Sendable (VoiceSttLoadProgress) -> Void
    ) async throws {
        self.savedOnPartial = onPartial
        self.savedOnEou = onEou
        if self.loaded { return }
        if let existing = self.loadTask {
            try await existing.value
            return
        }
        let chunkSize = self.chunkSize
        let debounceMs = self.eouDebounceMs
        let task = Task<Void, Error> { [weak self] in
            guard let self else { return }
            try await self.performLoad(
                chunkSize: chunkSize,
                debounceMs: debounceMs,
                onPartial: onPartial,
                onEou: onEou,
                onProgress: onProgress
            )
        }
        self.loadTask = task
        do {
            try await task.value
        } catch {
            self.loadTask = nil
            await self.stopCallbackForwarding()
            throw error
        }
    }

    private func performLoad(
        chunkSize: StreamingChunkSize,
        debounceMs: Int,
        onPartial: @escaping @Sendable (String) async -> Void,
        onEou: @escaping @Sendable (String) async -> Void,
        onProgress: @escaping @Sendable (VoiceSttLoadProgress) -> Void
    ) async throws {
        let mgr = StreamingEouAsrManager(
            chunkSize: chunkSize,
            eouDebounceMs: debounceMs
        )
        self.savedOnPartial = onPartial
        self.savedOnEou = onEou
        let callbackContinuation = self.startCallbackForwarding()
        await self.installCallbacks(on: mgr, continuation: callbackContinuation)
        try await mgr.loadModels(
            progressHandler: { onProgress(voiceSttProgress($0)) }
        )
        self.manager = mgr
        self.loaded = true
        self.loadTask = nil
        self.logger.info("FluidAudioStt loaded chunk=\(chunkSize.durationMs)ms eou=\(debounceMs)ms")
    }

    func cancelLoading() async {
        guard let task = self.loadTask else { return }
        task.cancel()
        _ = try? await task.value
        self.loadTask = nil
    }

    func acceptSamples(_ samples: [Float]) async throws {
        guard let mgr = self.manager else {
            throw FluidAudioSttError.notLoaded
        }
        guard let inputFormat,
              let buffer = AVAudioPCMBuffer(
                  pcmFormat: inputFormat,
                  frameCapacity: AVAudioFrameCount(samples.count)
              ),
              let data = buffer.floatChannelData?[0]
        else {
            throw FluidAudioSttError.audioBufferCreationFailed
        }
        buffer.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { source in
            guard let base = source.baseAddress else { return }
            data.update(from: base, count: source.count)
        }
        try await mgr.appendAudio(buffer)
        try await mgr.processBufferedAudio()
    }

    func finish() async throws -> String {
        guard let mgr = self.manager else { return "" }
        let transcript = try await mgr.finish()
        await withCheckedContinuation { continuation in
            guard let callbackContinuation = self.callbackContinuation,
                  self.callbackTask != nil
            else {
                continuation.resume()
                return
            }
            callbackContinuation.yield(.barrier(continuation))
        }
        return transcript
    }

    func reset() async throws {
        guard let mgr = self.manager else { return }
        await self.stopCallbackForwarding()
        await mgr.reset()
        await self.commitLayer.reset()
        let callbackContinuation = self.startCallbackForwarding()
        await self.installCallbacks(on: mgr, continuation: callbackContinuation)
    }

    func cleanup() async {
        await self.cancelLoading()
        await self.stopCallbackForwarding()
        guard let mgr = self.manager else { return }
        await mgr.cleanup()
        self.manager = nil
        self.loaded = false
    }

    private func startCallbackForwarding() -> AsyncStream<VoiceSttCallbackEvent>.Continuation {
        let (stream, continuation) = AsyncStream.makeStream(
            of: VoiceSttCallbackEvent.self,
            bufferingPolicy: .bufferingNewest(16)
        )
        self.callbackContinuation = continuation
        self.callbackTask = Task { [weak self] in
            for await event in stream {
                guard let self, !Task.isCancelled else { return }
                switch event {
                case let .partial(text):
                    await self.handleRawPartial(text)
                case let .endOfUtterance(text):
                    await self.handleRawEou(text)
                case let .barrier(continuation):
                    continuation.resume()
                }
            }
        }
        return continuation
    }

    private func installCallbacks(
        on manager: StreamingEouAsrManager,
        continuation: AsyncStream<VoiceSttCallbackEvent>.Continuation
    ) async {
        await manager.setPartialCallback { [weak self] text in
            guard self != nil else { return }
            continuation.yield(.partial(text))
        }
        await manager.setEouCallback { [weak self] text in
            guard self != nil else { return }
            continuation.yield(.endOfUtterance(text))
        }
    }

    private func stopCallbackForwarding() async {
        self.callbackContinuation?.finish()
        self.callbackContinuation = nil
        let callbackTask = self.callbackTask
        self.callbackTask = nil
        callbackTask?.cancel()
        await callbackTask?.value
    }
}

enum FluidAudioSttError: Error {
    case notLoaded
    case audioBufferCreationFailed
}

public extension StreamingEouAsrManager {
    func updateEouDebounceMs(_ value: Int) {
        self.eouDebounceMs = value
    }
}
