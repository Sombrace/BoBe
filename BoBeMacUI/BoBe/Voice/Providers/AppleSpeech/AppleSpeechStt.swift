@preconcurrency import AVFoundation
import Foundation
import OSLog
import Speech

actor AppleSpeechStt: VoiceSttEngine {
    private let logger = Logger(subsystem: "com.bobe.app", category: "AppleSpeechStt")
    private let requestedLocale: Locale
    private var eouDebounce: Duration

    private var resolvedLocale: Locale?
    private var transcriber: SpeechTranscriber?
    private var detector: SpeechDetector?
    private var analyzer: SpeechAnalyzer?
    private var analyzerInputContinuation: AsyncStream<AnalyzerInput>.Continuation?
    private var analysisTask: Task<Void, Never>?
    private var transcriberTask: Task<Void, Never>?
    private var detectorTask: Task<Void, Never>?
    private var eouTask: Task<Void, Never>?
    private var loadTask: Task<Void, Error>?

    private var analyzerFormat: AVAudioFormat?
    private var inputConverter: AVAudioConverter?
    private var savedOnPartial: (@Sendable (String) async -> Void)?
    private var savedOnEou: (@Sendable (String) async -> Void)?
    private var savedOnFailure: (@Sendable (String) async -> Void)?
    private var committedText = ""
    private var volatileText = ""
    private var loaded = false
    private var finishing = false
    private var generation: UInt64 = 0

    private let inputFormat = AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: 16_000,
        channels: 1,
        interleaved: false
    )

    init(
        language: String,
        eouDebounceMs: Int = PauseSensitivityMs.balanced
    ) {
        self.requestedLocale = Locale(identifier: language)
        self.eouDebounce = .milliseconds(max(100, min(5_000, eouDebounceMs)))
    }

    func setEouDebounceMs(_ value: Int) {
        self.eouDebounce = .milliseconds(max(100, min(5_000, value)))
    }

    func loadModels(
        onPartial: @escaping @Sendable (String) async -> Void,
        onEou: @escaping @Sendable (String) async -> Void,
        onFailure: @escaping @Sendable (String) async -> Void,
        onProgress: @escaping @Sendable (VoiceSttLoadProgress) -> Void
    ) async throws {
        self.savedOnPartial = onPartial
        self.savedOnEou = onEou
        self.savedOnFailure = onFailure
        if self.loaded { return }
        if let loadTask = self.loadTask {
            try await loadTask.value
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
            await self.stopRuntime()
            throw error
        }
    }

    func cancelLoading() async {
        guard let loadTask = self.loadTask else { return }
        loadTask.cancel()
        _ = try? await loadTask.value
        self.loadTask = nil
    }

    func acceptSamples(_ samples: [Float]) async throws {
        guard self.loaded,
              let inputFormat,
              let analyzerFormat,
              let continuation = self.analyzerInputContinuation
        else {
            throw AppleSpeechSttError.notLoaded
        }
        guard !samples.isEmpty else { return }
        guard let source = AVAudioPCMBuffer(
            pcmFormat: inputFormat,
            frameCapacity: AVAudioFrameCount(samples.count)
        ), let channel = source.floatChannelData?[0]
        else {
            throw AppleSpeechSttError.audioBufferCreationFailed
        }
        source.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { pointer in
            guard let baseAddress = pointer.baseAddress else { return }
            channel.update(from: baseAddress, count: pointer.count)
        }

        let buffer: AVAudioPCMBuffer
        if let inputConverter = self.inputConverter {
            let ratio = analyzerFormat.sampleRate / inputFormat.sampleRate
            let capacity = AVAudioFrameCount(ceil(Double(samples.count) * ratio) + 32)
            guard let converted = AVAudioPCMBuffer(
                pcmFormat: analyzerFormat,
                frameCapacity: capacity
            ) else {
                throw AppleSpeechSttError.audioBufferCreationFailed
            }
            let conversion = convertSingleBuffer(inputConverter, source: source, into: converted)
            guard conversion.status != .error, conversion.error == nil else {
                throw AppleSpeechSttError.audioConversionFailed(
                    conversion.error?.localizedDescription ?? "unknown conversion error"
                )
            }
            buffer = converted
        } else {
            buffer = source
        }

        guard case .terminated = continuation.yield(AnalyzerInput(buffer: buffer)) else { return }
        throw AppleSpeechSttError.analyzerStopped
    }

    func finish() async throws -> String {
        guard self.loaded, let analyzer = self.analyzer else { return "" }
        self.finishing = true
        self.eouTask?.cancel()
        self.eouTask = nil
        self.analyzerInputContinuation?.finish()
        self.analyzerInputContinuation = nil
        do {
            try await analyzer.finalizeAndFinishThroughEndOfInput()
        } catch {
            await self.stopRuntime()
            throw error
        }
        await self.awaitRuntimeTasks()
        let transcript = self.currentTranscript
        self.analyzer = nil
        self.analyzerFormat = nil
        self.inputConverter = nil
        self.clearTranscript()
        self.finishing = false
        return transcript
    }

    func reset() async throws {
        guard self.loaded,
              let transcriber = self.transcriber,
              let detector = self.detector
        else { return }
        await self.stopRuntime()
        self.clearTranscript()
        try await self.prepareRuntime(transcriber: transcriber, detector: detector)
    }

    func cleanup() async {
        await self.cancelLoading()
        await self.stopRuntime()
        if let resolvedLocale = self.resolvedLocale {
            _ = await AssetInventory.release(reservedLocale: resolvedLocale)
        }
        self.resolvedLocale = nil
        self.transcriber = nil
        self.detector = nil
        self.analyzerFormat = nil
        self.inputConverter = nil
        self.savedOnPartial = nil
        self.savedOnEou = nil
        self.savedOnFailure = nil
        self.clearTranscript()
        self.loaded = false
    }

    private func performLoad(
        onProgress: @escaping @Sendable (VoiceSttLoadProgress) -> Void
    ) async throws {
        await self.stopRuntime()
        onProgress(VoiceSttLoadProgress(fractionCompleted: 0, phase: .listing))
        guard await appleSpeechAuthorizationStatus() == .authorized else {
            throw AppleSpeechSttError.permissionDenied
        }
        guard SpeechTranscriber.isAvailable else {
            throw AppleSpeechSttError.unavailable
        }
        guard let locale = await SpeechTranscriber.supportedLocale(
            equivalentTo: self.requestedLocale
        ) else {
            throw AppleSpeechSttError.unsupportedLocale(self.requestedLocale.identifier)
        }

        let transcriber = SpeechTranscriber(locale: locale, preset: .progressiveTranscription)
        let detector = SpeechDetector(
            detectionOptions: .init(sensitivityLevel: .medium),
            reportResults: true
        )
        let modules: [any SpeechModule] = [transcriber, detector]
        switch await AssetInventory.status(forModules: modules) {
        case .unsupported:
            throw AppleSpeechSttError.unsupportedLocale(locale.identifier)
        case .supported:
            guard let request = try await AssetInventory.assetInstallationRequest(
                supporting: modules
            ) else {
                throw AppleSpeechSttError.assetRequestUnavailable
            }
            onProgress(VoiceSttLoadProgress(
                fractionCompleted: request.progress.fractionCompleted,
                phase: .downloading(completedFiles: 0, totalFiles: 1)
            ))
            try await request.downloadAndInstall()
            onProgress(VoiceSttLoadProgress(
                fractionCompleted: 1,
                phase: .downloading(completedFiles: 1, totalFiles: 1)
            ))
        case .downloading:
            guard let request = try await AssetInventory.assetInstallationRequest(
                supporting: modules
            ) else {
                throw AppleSpeechSttError.assetRequestUnavailable
            }
            onProgress(VoiceSttLoadProgress(
                fractionCompleted: request.progress.fractionCompleted,
                phase: .downloading(completedFiles: 0, totalFiles: 1)
            ))
            try await request.downloadAndInstall()
        case .installed:
            break
        @unknown default:
            throw AppleSpeechSttError.unknownAssetStatus
        }

        let needsReservation = self.resolvedLocale != locale
        if needsReservation {
            guard try await AssetInventory.reserve(locale: locale) else {
                throw AppleSpeechSttError.localeReservationFailed(locale.identifier)
            }
        }
        do {
            self.resolvedLocale = locale
            self.transcriber = transcriber
            self.detector = detector
            try await self.prepareRuntime(transcriber: transcriber, detector: detector)
            self.loaded = true
            self.loadTask = nil
            self.logger.info("Apple Speech loaded locale=\(locale.identifier, privacy: .public)")
        } catch {
            if needsReservation {
                _ = await AssetInventory.release(reservedLocale: locale)
                self.resolvedLocale = nil
            }
            throw error
        }
    }

    private func prepareRuntime(
        transcriber: SpeechTranscriber,
        detector: SpeechDetector
    ) async throws {
        let modules: [any SpeechModule] = [transcriber, detector]
        guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(
            compatibleWith: modules
        ) else {
            throw AppleSpeechSttError.noCompatibleAudioFormat
        }
        guard let inputFormat = self.inputFormat else {
            throw AppleSpeechSttError.audioBufferCreationFailed
        }
        let converter: AVAudioConverter?
        if format == inputFormat {
            converter = nil
        } else {
            guard let created = AVAudioConverter(from: inputFormat, to: format) else {
                throw AppleSpeechSttError.audioConverterCreationFailed
            }
            converter = created
        }

        let analyzer = SpeechAnalyzer(
            modules: modules,
            options: .init(priority: .userInitiated, modelRetention: .processLifetime)
        )
        try await analyzer.prepareToAnalyze(in: format)

        let (inputStream, continuation) = AsyncStream.makeStream(
            of: AnalyzerInput.self,
            bufferingPolicy: .bufferingNewest(32)
        )
        self.generation &+= 1
        let generation = self.generation
        self.analyzer = analyzer
        self.analyzerFormat = format
        self.inputConverter = converter
        self.analyzerInputContinuation = continuation
        self.finishing = false

        self.transcriberTask = Task { [weak self, transcriber] in
            do {
                for try await result in transcriber.results {
                    guard !Task.isCancelled else { return }
                    await self?.handleTranscription(result, generation: generation)
                }
            } catch is CancellationError {
                return
            } catch {
                await self?.handleRuntimeFailure(error, generation: generation)
            }
        }
        self.detectorTask = Task { [weak self, detector] in
            do {
                for try await result in detector.results {
                    guard !Task.isCancelled else { return }
                    await self?.handleDetection(result, generation: generation)
                }
            } catch is CancellationError {
                return
            } catch {
                await self?.handleRuntimeFailure(error, generation: generation)
            }
        }
        self.analysisTask = Task { [weak self, analyzer] in
            do {
                try await analyzer.start(inputSequence: inputStream)
            } catch is CancellationError {
                return
            } catch {
                await self?.handleRuntimeFailure(error, generation: generation)
            }
        }
    }

    private func handleTranscription(
        _ result: SpeechTranscriber.Result,
        generation: UInt64
    ) async {
        guard generation == self.generation else { return }
        let text = String(result.text.characters)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        if result.isFinal {
            self.committedText = mergeSpeechTranscript(
                committed: self.committedText,
                candidate: text
            )
            self.volatileText = ""
        } else {
            self.volatileText = text
        }
        await self.savedOnPartial?(self.currentTranscript)
    }

    private func handleDetection(
        _ result: SpeechDetector.Result,
        generation: UInt64
    ) {
        guard generation == self.generation, !self.finishing else { return }
        self.eouTask?.cancel()
        self.eouTask = nil
        guard !result.speechDetected else { return }
        let debounce = self.eouDebounce
        self.eouTask = Task { [weak self] in
            do {
                try await Task.sleep(for: debounce)
                await self?.emitEndOfUtterance(generation: generation)
            } catch {
                return
            }
        }
    }

    private func emitEndOfUtterance(generation: UInt64) async {
        guard generation == self.generation,
              !self.finishing,
              !self.currentTranscript.isEmpty,
              let transcriber = self.transcriber,
              let detector = self.detector
        else { return }
        let transcript = self.currentTranscript
        self.eouTask = nil
        await self.stopRuntime()
        self.clearTranscript()
        await self.savedOnEou?(transcript)
        guard self.loaded else { return }
        do {
            try await self.prepareRuntime(transcriber: transcriber, detector: detector)
        } catch {
            self.loaded = false
            let message = error.localizedDescription
            self.logger.error(
                "Apple Speech turn reset failed: \(message, privacy: .public)"
            )
            await self.savedOnFailure?(message)
        }
    }

    private func handleRuntimeFailure(_ error: Error, generation: UInt64) {
        guard generation == self.generation, !self.finishing else { return }
        let message = error.localizedDescription
        self.logger.error(
            "Apple Speech runtime failed: \(message, privacy: .public)"
        )
        self.loaded = false
        self.generation &+= 1
        let failedGeneration = self.generation
        self.analyzerInputContinuation?.finish()
        self.analyzerInputContinuation = nil
        Task { [weak self] in
            guard let self else { return }
            await self.finishRuntimeFailure(
                generation: failedGeneration,
                message: message
            )
        }
    }

    private func finishRuntimeFailure(
        generation: UInt64,
        message: String
    ) async {
        guard generation == self.generation else { return }
        await self.stopRuntime()
        await self.savedOnFailure?(message)
    }

    private func stopRuntime() async {
        self.generation &+= 1
        self.finishing = true
        self.eouTask?.cancel()
        self.eouTask = nil
        self.analyzerInputContinuation?.finish()
        self.analyzerInputContinuation = nil
        if let analyzer = self.analyzer {
            await analyzer.cancelAndFinishNow()
        }
        self.analysisTask?.cancel()
        self.transcriberTask?.cancel()
        self.detectorTask?.cancel()
        await self.awaitRuntimeTasks()
        self.analyzer = nil
        self.analyzerFormat = nil
        self.inputConverter = nil
        self.finishing = false
    }

    private func awaitRuntimeTasks() async {
        let analysisTask = self.analysisTask
        let transcriberTask = self.transcriberTask
        let detectorTask = self.detectorTask
        self.analysisTask = nil
        self.transcriberTask = nil
        self.detectorTask = nil
        await analysisTask?.value
        await transcriberTask?.value
        await detectorTask?.value
    }

    private var currentTranscript: String {
        mergeSpeechTranscript(committed: self.committedText, candidate: self.volatileText)
    }

    private func clearTranscript() {
        self.committedText = ""
        self.volatileText = ""
    }
}

func appleSpeechAuthorizationStatus() async -> SFSpeechRecognizerAuthorizationStatus {
    let status = SFSpeechRecognizer.authorizationStatus()
    guard status == .notDetermined else { return status }
    return await withCheckedContinuation { continuation in
        SFSpeechRecognizer.requestAuthorization { grantedStatus in
            continuation.resume(returning: grantedStatus)
        }
    }
}

func mergeSpeechTranscript(committed: String, candidate: String) -> String {
    let committed = committed.trimmingCharacters(in: .whitespacesAndNewlines)
    let candidate = candidate.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !committed.isEmpty else { return candidate }
    guard !candidate.isEmpty else { return committed }
    if candidate.hasPrefix(committed) { return candidate }
    if committed.hasSuffix(candidate) { return committed }
    return "\(committed) \(candidate)"
}

enum AppleSpeechSttError: LocalizedError {
    case permissionDenied
    case unavailable
    case unsupportedLocale(String)
    case assetRequestUnavailable
    case localeReservationFailed(String)
    case unknownAssetStatus
    case noCompatibleAudioFormat
    case audioConverterCreationFailed
    case audioBufferCreationFailed
    case audioConversionFailed(String)
    case analyzerStopped
    case notLoaded

    var errorDescription: String? {
        switch self {
        case .permissionDenied:
            "Speech recognition permission is required for Apple Speech."
        case .unavailable:
            "Apple Speech is unavailable on this Mac."
        case let .unsupportedLocale(locale):
            "Apple Speech does not support \(locale)."
        case .assetRequestUnavailable:
            "Apple Speech could not create a model download request."
        case let .localeReservationFailed(locale):
            "Apple Speech could not reserve its \(locale) model."
        case .unknownAssetStatus:
            "Apple Speech returned an unknown model status."
        case .noCompatibleAudioFormat:
            "Apple Speech did not provide a compatible audio format."
        case .audioConverterCreationFailed:
            "Apple Speech audio conversion could not be configured."
        case .audioBufferCreationFailed:
            "Apple Speech could not allocate an audio buffer."
        case let .audioConversionFailed(reason):
            "Apple Speech audio conversion failed: \(reason)"
        case .analyzerStopped:
            "Apple Speech stopped accepting audio."
        case .notLoaded:
            "Apple Speech is not ready."
        }
    }
}
