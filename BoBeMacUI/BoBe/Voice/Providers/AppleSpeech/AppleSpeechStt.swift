@preconcurrency import AVFoundation
import Foundation
import OSLog
import Speech

nonisolated struct VoiceSttLoadProgress: Sendable, Equatable {
    let fractionCompleted: Double

    var percent: Int {
        max(0, min(100, Int((self.fractionCompleted * 100).rounded())))
    }
}

nonisolated struct SpeechRecognitionModel: Sendable, Equatable {
    let locale: Locale
    let dictation: Bool

    static func resolve(for language: String) async -> Self? {
        let requested = Locale(identifier: language)
        if SpeechTranscriber.isAvailable,
           let locale = SpeechRecognitionModule.closest(to: requested, in: await SpeechTranscriber.supportedLocales) {
            return Self(locale: locale, dictation: false)
        }
        if let locale = SpeechRecognitionModule.closest(to: requested, in: await DictationTranscriber.supportedLocales) {
            return Self(locale: locale, dictation: true)
        }
        return nil
    }

    func makeModule() -> SpeechRecognitionModule {
        guard self.dictation else {
            return .transcriber(SpeechTranscriber(
                locale: self.locale,
                transcriptionOptions: [],
                reportingOptions: [.volatileResults, .fastResults],
                attributeOptions: []
            ))
        }
        return .dictation(DictationTranscriber(
            locale: self.locale,
            contentHints: [.shortForm],
            transcriptionOptions: [.punctuation],
            reportingOptions: [.volatileResults, .frequentFinalization],
            attributeOptions: []
        ))
    }
}

nonisolated enum SpeechRecognitionModule: Sendable {
    case transcriber(SpeechTranscriber)
    case dictation(DictationTranscriber)

    var module: any SpeechModule {
        switch self {
        case let .transcriber(module): module
        case let .dictation(module): module
        }
    }

    static func closest(to requested: Locale, in supported: [Locale]) -> Locale? {
        let candidates = supported.filter { $0.language.languageCode == requested.language.languageCode }
        let region = requested.region ?? Locale.Language(identifier: requested.language.maximalIdentifier).region
        return candidates.first { $0.region == region } ?? candidates.first
    }

    func forEachResult(_ body: (String, Bool) async -> Void) async throws {
        switch self {
        case let .transcriber(module):
            for try await result in module.results {
                await body(String(result.text.characters), result.isFinal)
            }
        case let .dictation(module):
            for try await result in module.results {
                await body(String(result.text.characters), result.isFinal)
            }
        }
    }
}

actor AppleSpeechStt {
    private static let heldSampleLimit = VoiceWire.captureSampleRate * 10

    private let logger = Logger(subsystem: "com.bobe.app", category: "AppleSpeechStt")
    let language: String
    private var pauseDuration: Duration

    private var model: SpeechRecognitionModel?
    private var reservedLocale: Locale?
    private var analyzer: SpeechAnalyzer?
    private var analyzerInput: AsyncStream<AnalyzerInput>.Continuation?
    private var analyzerFormat: AVAudioFormat?
    private var inputConverter: AVAudioConverter?
    private var runtime: Task<Void, Never>?
    private var loadTask: Task<Void, any Error>?

    private var onPartial: (@Sendable (String) async -> Void)?
    private var onEou: (@Sendable (String) async -> Void)?
    private var onFailure: (@Sendable (String) async -> Void)?
    private var committedText = ""
    private var volatileText = ""
    private var pause = SpeechPause()
    private var heldSamples: [Float] = []
    private var finishing = false
    private var generation: UInt64 = 0

    private let inputFormat = AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: Double(VoiceWire.captureSampleRate),
        channels: 1,
        interleaved: false
    )

    init(language: String, pauseMs: Int = PauseSensitivityMs.balanced) {
        self.language = language
        self.pauseDuration = Self.pause(ms: pauseMs)
    }

    var isLoaded: Bool {
        self.model != nil
    }

    func setPauseMs(_ value: Int) {
        self.pauseDuration = Self.pause(ms: value)
    }

    nonisolated static func assetsInstalled(for language: String) async -> Bool {
        guard let model = await SpeechRecognitionModel.resolve(for: language) else { return false }
        return await AssetInventory.status(forModules: [model.makeModule().module]) == .installed
    }

    func load(
        onPartial: @escaping @Sendable (String) async -> Void,
        onEou: @escaping @Sendable (String) async -> Void,
        onFailure: @escaping @Sendable (String) async -> Void,
        onProgress: @escaping @Sendable (VoiceSttLoadProgress) -> Void
    ) async throws {
        self.onPartial = onPartial
        self.onEou = onEou
        self.onFailure = onFailure
        if self.model != nil { return }
        if let loadTask = self.loadTask {
            try await loadTask.value
            return
        }
        let task = Task<Void, any Error>(name: "Apple Speech load \(self.language)") {
            try await self.performLoad(onProgress: onProgress)
        }
        self.loadTask = task
        defer { self.loadTask = nil }
        try await task.value
    }

    func cancelLoading() async {
        guard let loadTask = self.loadTask else { return }
        loadTask.cancel()
        _ = try? await loadTask.value
    }

    func acceptSamples(_ samples: [Float]) async throws(AppleSpeechSttError) {
        guard self.model != nil else { throw .notLoaded }
        guard !samples.isEmpty else { return }
        guard let analyzerInput = self.analyzerInput, !self.finishing else {
            self.hold(samples)
            return
        }
        try self.feed(samples, to: analyzerInput)
        self.pause.observe(samples)
        if self.pause.reached(self.pauseDuration), !self.transcript.isEmpty {
            await self.endUtterance()
        }
    }

    func finish() async -> String {
        guard self.model != nil else { return "" }
        let transcript = await self.finalizeUtterance()
        await self.restartRuntime()
        return transcript
    }

    func reset() async throws(AppleSpeechSttError) {
        guard self.model != nil else { return }
        await self.stopRuntime()
        self.heldSamples.removeAll()
        try await self.startRuntime()
    }

    func cleanup() async {
        await self.cancelLoading()
        await self.stopRuntime()
        self.heldSamples.removeAll()
        if let reservedLocale = self.reservedLocale {
            await AssetInventory.release(reservedLocale: reservedLocale)
        }
        self.reservedLocale = nil
        self.model = nil
        self.onPartial = nil
        self.onEou = nil
        self.onFailure = nil
    }

    private var transcript: String {
        (self.committedText + self.volatileText).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func performLoad(onProgress: @escaping @Sendable (VoiceSttLoadProgress) -> Void) async throws {
        onProgress(VoiceSttLoadProgress(fractionCompleted: 0))
        guard let model = await SpeechRecognitionModel.resolve(for: self.language) else {
            throw AppleSpeechSttError.unsupportedLocale(self.language)
        }
        let locale = model.locale
        try await Self.installAssets(for: [model.makeModule().module], onProgress: onProgress)
        try Task.checkCancellation()
        if self.reservedLocale != locale {
            try await Self.reserve(locale)
            self.reservedLocale = locale
        }
        self.model = model
        do {
            try await self.startRuntime()
        } catch {
            self.model = nil
            throw error
        }
        onProgress(VoiceSttLoadProgress(fractionCompleted: 1))
        self.logger.info("Apple Speech loaded locale=\(locale.identifier, privacy: .public)")
    }

    nonisolated private static func installAssets(
        for modules: [any SpeechModule],
        onProgress: @escaping @Sendable (VoiceSttLoadProgress) -> Void
    ) async throws {
        switch await AssetInventory.status(forModules: modules) {
        case .installed:
            return
        case .unsupported:
            throw AppleSpeechSttError.assetsUnsupported
        case .supported, .downloading:
            break
        @unknown default:
            throw AppleSpeechSttError.assetsUnsupported
        }
        guard let request = try await AssetInventory.assetInstallationRequest(supporting: modules) else { return }
        let observation = request.progress.observe(\.fractionCompleted, options: [.initial, .new]) { progress, _ in
            onProgress(VoiceSttLoadProgress(fractionCompleted: progress.fractionCompleted))
        }
        defer { observation.invalidate() }
        try await request.downloadAndInstall()
    }

    nonisolated private static func reserve(_ locale: Locale) async throws {
        let reserved = await AssetInventory.reservedLocales
        if reserved.contains(locale) { return }
        if reserved.count >= AssetInventory.maximumReservedLocales, let oldest = reserved.first {
            await AssetInventory.release(reservedLocale: oldest)
        }
        do {
            try await AssetInventory.reserve(locale: locale)
        } catch {
            throw AppleSpeechSttError.localeReservationFailed(locale.identifier)
        }
    }

    private func startRuntime() async throws(AppleSpeechSttError) {
        guard let model = self.model, let inputFormat = self.inputFormat else { throw .notLoaded }
        let recognition = model.makeModule()
        self.generation &+= 1
        let generation = self.generation
        let modules = [recognition.module]
        guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(
            compatibleWith: modules,
            considering: inputFormat
        ) else {
            throw .noCompatibleAudioFormat
        }
        let converter: AVAudioConverter?
        if format == inputFormat {
            converter = nil
        } else {
            guard let created = AVAudioConverter(from: inputFormat, to: format) else {
                throw .audioConverterCreationFailed
            }
            converter = created
        }
        let analyzer = SpeechAnalyzer(
            modules: modules,
            options: .init(priority: .userInitiated, modelRetention: .processLifetime)
        )
        do {
            let context = AnalysisContext()
            context.contextualStrings[.general] = ["BoBe"]
            try await analyzer.setContext(context)
            try await analyzer.prepareToAnalyze(in: format)
        } catch {
            throw .analyzerUnavailable(error.localizedDescription)
        }
        guard generation == self.generation else {
            await analyzer.cancelAndFinishNow()
            return
        }
        let (inputStream, analyzerInput) = AsyncStream.makeStream(of: AnalyzerInput.self)
        self.analyzer = analyzer
        self.analyzerFormat = format
        self.inputConverter = converter
        self.analyzerInput = analyzerInput
        self.runtime = Task(name: "Apple Speech \(self.language)") { [weak self] in
            do {
                try await withThrowingDiscardingTaskGroup { group in
                    group.addTask {
                        try await analyzer.start(inputSequence: inputStream)
                    }
                    group.addTask {
                        try await recognition.forEachResult { text, isFinal in
                            await self?.receive(text, isFinal: isFinal, generation: generation)
                        }
                    }
                }
            } catch is CancellationError {
                return
            } catch {
                await self?.runtimeFailed(error, generation: generation)
            }
        }
        let held = self.heldSamples
        self.heldSamples.removeAll()
        if !held.isEmpty {
            try self.feed(held, to: analyzerInput)
            self.pause.observe(held)
        }
    }

    private func restartRuntime() async {
        do {
            try await self.startRuntime()
        } catch {
            self.model = nil
            self.logger.error("Apple Speech restart failed: \(error.localizedDescription, privacy: .public)")
            await self.onFailure?(error.localizedDescription)
        }
    }

    private func stopRuntime() async {
        self.generation &+= 1
        self.analyzerInput?.finish()
        self.analyzerInput = nil
        if let analyzer = self.analyzer {
            await analyzer.cancelAndFinishNow()
        }
        let runtime = self.runtime
        runtime?.cancel()
        await runtime?.value
        self.discardRuntime()
    }

    private func discardRuntime() {
        self.analyzer = nil
        self.runtime = nil
        self.analyzerFormat = nil
        self.inputConverter = nil
        self.committedText = ""
        self.volatileText = ""
        self.pause.reset()
    }

    private func finalizeUtterance() async -> String {
        guard let analyzer = self.analyzer, !self.finishing else { return self.transcript }
        self.finishing = true
        defer { self.finishing = false }
        self.analyzerInput?.finish()
        self.analyzerInput = nil
        do {
            try await analyzer.finalizeAndFinishThroughEndOfInput()
        } catch {
            self.logger.warning("Apple Speech finalize failed: \(error.localizedDescription, privacy: .public)")
        }
        await self.runtime?.value
        let transcript = self.transcript
        self.generation &+= 1
        self.discardRuntime()
        return transcript
    }

    private func endUtterance() async {
        let transcript = await self.finalizeUtterance()
        if !transcript.isEmpty {
            await self.onEou?(transcript)
        }
        await self.restartRuntime()
    }

    private func receive(_ text: String, isFinal: Bool, generation: UInt64) async {
        guard generation == self.generation else { return }
        if isFinal {
            self.committedText += text
            self.volatileText = ""
        } else {
            self.volatileText = text
        }
        let transcript = self.transcript
        guard !transcript.isEmpty else { return }
        await self.onPartial?(transcript)
    }

    private func runtimeFailed(_ error: any Error, generation: UInt64) async {
        guard generation == self.generation, !self.finishing else { return }
        let message = error.localizedDescription
        self.logger.error("Apple Speech runtime failed: \(message, privacy: .public)")
        await self.stopRuntime()
        self.model = nil
        await self.onFailure?(message)
    }

    private func hold(_ samples: [Float]) {
        self.heldSamples.append(contentsOf: samples)
        let overflow = self.heldSamples.count - Self.heldSampleLimit
        if overflow > 0 {
            self.heldSamples.removeFirst(overflow)
        }
    }

    private func feed(
        _ samples: [Float],
        to analyzerInput: AsyncStream<AnalyzerInput>.Continuation
    ) throws(AppleSpeechSttError) {
        guard let inputFormat = self.inputFormat,
              let analyzerFormat = self.analyzerFormat,
              let source = AVAudioPCMBuffer(pcmFormat: inputFormat, frameCapacity: AVAudioFrameCount(samples.count)),
              let channel = source.floatChannelData?[0]
        else {
            throw .audioBufferCreationFailed
        }
        source.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { pointer in
            guard let baseAddress = pointer.baseAddress else { return }
            channel.update(from: baseAddress, count: pointer.count)
        }
        let buffer: AVAudioPCMBuffer
        if let inputConverter = self.inputConverter {
            let ratio = analyzerFormat.sampleRate / inputFormat.sampleRate
            guard let converted = AVAudioPCMBuffer(
                pcmFormat: analyzerFormat,
                frameCapacity: AVAudioFrameCount((Double(samples.count) * ratio).rounded(.up)) + 32
            ) else {
                throw .audioBufferCreationFailed
            }
            let conversion = convertSingleBuffer(inputConverter, source: source, into: converted)
            guard conversion.status != .error, conversion.error == nil else {
                throw .audioConversionFailed(conversion.error?.localizedDescription ?? "unknown conversion error")
            }
            buffer = converted
        } else {
            buffer = source
        }
        if case .terminated = analyzerInput.yield(AnalyzerInput(buffer: buffer)) {
            throw .analyzerStopped
        }
    }

    nonisolated private static func pause(ms: Int) -> Duration {
        .milliseconds(max(100, min(5_000, ms)))
    }
}

nonisolated enum AppleSpeechSttError: LocalizedError {
    case unsupportedLocale(String)
    case assetsUnsupported
    case localeReservationFailed(String)
    case noCompatibleAudioFormat
    case audioConverterCreationFailed
    case audioBufferCreationFailed
    case audioConversionFailed(String)
    case analyzerUnavailable(String)
    case analyzerStopped
    case notLoaded

    var errorDescription: String? {
        switch self {
        case let .unsupportedLocale(language):
            "This Mac can’t recognize speech in \(language)."
        case .assetsUnsupported:
            "This Mac can’t download the speech recognition model for this language."
        case let .localeReservationFailed(locale):
            "macOS couldn’t keep the \(locale) speech recognition model."
        case .noCompatibleAudioFormat:
            "Apple Speech did not provide a compatible audio format."
        case .audioConverterCreationFailed:
            "Apple Speech audio conversion could not be configured."
        case .audioBufferCreationFailed:
            "Apple Speech could not allocate an audio buffer."
        case let .audioConversionFailed(reason):
            "Apple Speech audio conversion failed: \(reason)"
        case let .analyzerUnavailable(reason):
            "Apple Speech could not start: \(reason)"
        case .analyzerStopped:
            "Apple Speech stopped accepting audio."
        case .notLoaded:
            "Apple Speech is not ready."
        }
    }
}
