import FluidAudio
import Foundation
import OSLog

@available(macOS 15, iOS 18, *)
actor FluidAudioNemotronStt: VoiceSttEngine {
    private let logger = Logger(subsystem: "com.bobe.app", category: "FluidAudioNemotronStt")
    private var languageCode = "zh"
    private var eouDelayMs = PauseSensitivityMs.balanced

    private var manager: StreamingNemotronMultilingualAsrManager?
    private var vadManager: VadManager?
    private var vadState: VadStreamState = .initial()
    private let commitLayer = PunctuationCommitLayer()

    private var onPartial: (@Sendable (String) async -> Void)?
    private var onEou: (@Sendable (String) async -> Void)?
    private var loadTask: Task<Void, Error>?
    private var eouTimer: Task<Void, Never>?
    private var callbackContinuation: AsyncStream<VoiceSttCallbackEvent>.Continuation?
    private var callbackTask: Task<Void, Never>?

    func setEouDelayMs(_ value: Int) {
        self.eouDelayMs = max(100, min(5_000, value))
    }

    func setLanguage(_ languageCode: String) async {
        self.languageCode = languageCode
        await self.manager?.setLanguage(Self.languageHint(for: languageCode))
    }

    func loadModels(
        onPartial: @escaping @Sendable (String) async -> Void,
        onEou: @escaping @Sendable (String) async -> Void,
        onFailure _: @escaping @Sendable (String) async -> Void,
        onProgress: @escaping @Sendable (VoiceSttLoadProgress) -> Void
    ) async throws {
        self.onPartial = onPartial
        self.onEou = onEou
        if self.manager != nil { return }
        if let existing = self.loadTask {
            try await existing.value
            return
        }

        let task = Task<Void, Error> { [weak self] in
            guard let self else { return }
            try await self.performLoad(onProgress: onProgress)
        }
        self.loadTask = task
        do {
            try await task.value
        } catch {
            self.loadTask = nil
            self.stopCallbackForwarding()
            throw error
        }
    }

    private func performLoad(
        onProgress: @escaping @Sendable (VoiceSttLoadProgress) -> Void
    ) async throws {
        let shared = try await StreamingNemotronMultilingualAsrManager
            .downloadAndPreloadShared(
                languageCode: "multilingual",
                chunkMs: VoiceSttTuning.nemotronChunkMs,
                progressHandler: { onProgress(voiceSttProgress($0)) }
            )
        try Task.checkCancellation()

        let manager = StreamingNemotronMultilingualAsrManager()
        try await manager.loadFromShared(shared)
        await manager.setLanguage(Self.languageHint(for: self.languageCode))
        await manager.setForcedPrefix(true)
        let callbackContinuation = self.startCallbackForwarding()
        await manager.setPartialCallback { [weak self] text in
            guard self != nil else { return }
            callbackContinuation.yield(.partial(text))
        }

        let vad = try await VadManager()
        let initialVadState = await vad.makeStreamState()

        self.manager = manager
        self.vadManager = vad
        self.vadState = initialVadState
        self.loadTask = nil
        self.logger.info(
            "FluidAudio Nemotron loaded for language \(self.languageCode, privacy: .public)"
        )
    }

    func cancelLoading() async {
        guard let task = self.loadTask else { return }
        task.cancel()
        _ = try? await task.value
        self.loadTask = nil
    }

    func acceptSamples(_ samples: [Float]) async throws {
        guard let manager = self.manager, let vad = self.vadManager else {
            throw FluidAudioNemotronSttError.notLoaded
        }
        guard !samples.isEmpty else { return }

        let vadResult = try await vad.processStreamingChunk(samples, state: self.vadState)
        self.vadState = vadResult.state
        if let event = vadResult.event {
            self.handleVadEvent(event)
        }

        _ = try await manager.process(samples: samples)
    }

    private func handleRawPartial(_ text: String) async {
        let update = await self.commitLayer.processPartialText(text)
        await self.onPartial?(update.totalText)
    }

    private func handleVadEvent(_ event: VadStreamEvent) {
        switch event.kind {
        case .speechStart:
            self.eouTimer?.cancel()
            self.eouTimer = nil
        case .speechEnd:
            self.scheduleEou()
        }
    }

    private func scheduleEou() {
        self.eouTimer?.cancel()
        let delayMs = self.eouDelayMs
        self.eouTimer = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(delayMs))
            guard !Task.isCancelled else { return }
            await self?.fireEou()
        }
    }

    private func fireEou() async {
        self.eouTimer = nil
        guard let manager = self.manager else { return }
        do {
            let transcript = try await manager.finish()
            let update = await self.commitLayer.processEOU()
            let final = update.committedText.isEmpty ? transcript : update.committedText
            await manager.reset()
            await self.commitLayer.reset()
            if let vad = self.vadManager {
                self.vadState = await vad.makeStreamState()
            }
            await self.onEou?(final)
        } catch {
            self.logger.error(
                "Nemotron finish failed: \(error.localizedDescription, privacy: .public)"
            )
            await self.onEou?("")
        }
    }

    func reset() async throws {
        self.eouTimer?.cancel()
        self.eouTimer = nil
        await self.manager?.reset()
        await self.commitLayer.reset()
        if let vad = self.vadManager {
            self.vadState = await vad.makeStreamState()
        }
    }

    func cleanup() async {
        await self.cancelLoading()
        self.eouTimer?.cancel()
        self.eouTimer = nil
        self.stopCallbackForwarding()
        await self.manager?.cleanup()
        self.manager = nil
        self.vadManager = nil
    }

    private func startCallbackForwarding() -> AsyncStream<VoiceSttCallbackEvent>.Continuation {
        self.stopCallbackForwarding()
        let (stream, continuation) = AsyncStream.makeStream(
            of: VoiceSttCallbackEvent.self,
            bufferingPolicy: .bufferingNewest(16)
        )
        self.callbackContinuation = continuation
        self.callbackTask = Task { [weak self] in
            for await event in stream {
                guard let self, !Task.isCancelled else { return }
                if case let .partial(text) = event {
                    await self.handleRawPartial(text)
                }
            }
        }
        return continuation
    }

    private func stopCallbackForwarding() {
        self.callbackContinuation?.finish()
        self.callbackContinuation = nil
        self.callbackTask?.cancel()
        self.callbackTask = nil
    }

    private static func languageHint(for code: String) -> String {
        switch code {
        case "zh": "zh-CN"
        case "es": "es-ES"
        case "el": "el-GR"
        case "ko": "ko-KR"
        case "ja": "ja-JP"
        default: code
        }
    }
}

@available(macOS 15, iOS 18, *)
nonisolated enum FluidAudioNemotronSttError: Error {
    case notLoaded
}
