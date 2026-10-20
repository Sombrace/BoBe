@preconcurrency import AVFoundation
import Observation
import Speech

@Observable
final class HoldToTalk {
    enum State: Equatable {
        case idle
        case preparing
        case listening
        case unavailable
    }

    private(set) var state: State = .idle
    private(set) var transcript = ""
    private(set) var level: Double = 0

    @ObservationIgnored private var listener: SpeechListener?
    @ObservationIgnored private var preparation: Task<Void, Never>?
    @ObservationIgnored private var engine: AVAudioEngine?
    @ObservationIgnored private var feed: SpeechFeed?
    @ObservationIgnored private var analyzer: SpeechAnalyzer?
    @ObservationIgnored private var results: Task<Bool, Never>?
    @ObservationIgnored private var finalized = ""
    @ObservationIgnored private var volatile = ""

    init(state: State = .idle) {
        self.state = state
    }

    func prepare() {
        guard self.listener == nil, self.preparation == nil, PhonePermissions.voice() == .granted else { return }
        self.preparation = Task {
            let listener = await SpeechListener.make(preferring: .current)
            self.listener = listener
            self.preparation = nil
            if self.state == .preparing {
                self.state = listener == nil ? .unavailable : .idle
            }
        }
    }

    func start() {
        guard self.state != .listening else { return }
        guard PhonePermissions.voice() == .granted else {
            self.state = .unavailable
            return
        }
        guard let listener = self.listener else {
            self.state = .preparing
            self.prepare()
            return
        }
        do {
            let session = AVAudioSession.sharedInstance()
            try session.setCategory(.record, mode: .measurement, options: .duckOthers)
            try session.setActive(true, options: .notifyOthersOnDeactivation)
            let engine = AVAudioEngine()
            let input = engine.inputNode
            let format = input.outputFormat(forBus: 0)
            let (inputs, continuation) = AsyncStream.makeStream(of: AnalyzerInput.self)
            let feed = try SpeechFeed(from: format, to: listener.format, into: continuation)
            let tap = Self.tap(feed: feed, owner: self)
            if #available(iOS 27, *) {
                try input.installAudioTap(onBus: 0, bufferSize: 1024, format: format) { buffer, time in
                    tap(AVAudioPCMBuffer(copying: buffer), time)
                }
            } else {
                input.installTap(onBus: 0, bufferSize: 1024, format: format, block: tap)
            }
            engine.prepare()
            try engine.start()
            let module = listener.module()
            self.finalized = ""
            self.volatile = ""
            self.transcript = ""
            self.level = 0
            self.engine = engine
            self.feed = feed
            self.results = self.collect(module)
            self.analyzer = SpeechAnalyzer(
                inputSequence: inputs,
                modules: [module],
                options: .init(priority: .userInitiated, modelRetention: .lingering)
            )
            self.state = .listening
        } catch {
            self.stopAudio()
            self.state = .unavailable
        }
    }

    func finish() async -> String {
        guard self.state == .listening else { return "" }
        self.stopAudio()
        self.feed?.end()
        try? await self.analyzer?.finalizeAndFinishThroughEndOfInput()
        let completed = await self.results?.value ?? false
        let text = (self.finalized + self.volatile).trimmingCharacters(in: .whitespacesAndNewlines)
        self.reset()
        if !completed {
            self.listener = nil
            if text.isEmpty { self.state = .unavailable }
        }
        return text
    }

    func cancel() {
        self.stopAudio()
        self.feed?.end()
        self.results?.cancel()
        if let analyzer = self.analyzer {
            Task { await analyzer.cancelAndFinishNow() }
        }
        self.reset()
    }

    func dismissUnavailable() {
        if self.state == .unavailable {
            self.state = .idle
        }
    }

    private func collect(_ module: any SpeechModule) -> Task<Bool, Never> {
        Task { [weak self] in
            do {
                if let transcriber = module as? SpeechTranscriber {
                    for try await result in transcriber.results {
                        self?.apply(result.text, isFinal: result.isFinal)
                    }
                } else if let transcriber = module as? DictationTranscriber {
                    for try await result in transcriber.results {
                        self?.apply(result.text, isFinal: result.isFinal)
                    }
                }
                return true
            } catch {
                return false
            }
        }
    }

    private func apply(_ text: AttributedString, isFinal: Bool) {
        let piece = String(text.characters)
        if isFinal {
            self.finalized += piece
            self.volatile = ""
        } else {
            self.volatile = piece
        }
        guard self.state == .listening else { return }
        self.transcript = (self.finalized + self.volatile).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func stopAudio() {
        if let engine {
            engine.inputNode.removeTap(onBus: 0)
            engine.stop()
        }
        self.engine = nil
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    private func reset() {
        self.feed = nil
        self.analyzer = nil
        self.results = nil
        self.finalized = ""
        self.volatile = ""
        self.level = 0
        self.transcript = ""
        self.state = .idle
    }

    private func receive(level: Double) {
        guard self.state == .listening else { return }
        self.level = level
    }

    nonisolated private static func tap(
        feed: SpeechFeed,
        owner: HoldToTalk
    ) -> @Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void {
        { [weak owner] buffer, _ in
            feed.append(buffer)
            let level = SpeechFeed.level(of: buffer)
            Task { @MainActor in
                owner?.receive(level: level)
            }
        }
    }
}

private struct SpeechListener: Sendable {
    let locale: Locale
    let dictation: Bool
    let format: AVAudioFormat

    func module() -> any SpeechModule {
        Self.module(locale: self.locale, dictation: self.dictation)
    }

    static func make(preferring preferred: Locale) async -> SpeechListener? {
        for requested in [preferred, Locale(identifier: "en-US")] {
            if SpeechTranscriber.isAvailable,
               let locale = self.closest(to: requested, in: await SpeechTranscriber.supportedLocales),
               let listener = await self.install(locale: locale, dictation: false) {
                return listener
            }
            if let locale = self.closest(to: requested, in: await DictationTranscriber.supportedLocales),
               let listener = await self.install(locale: locale, dictation: true) {
                return listener
            }
        }
        return nil
    }

    static func closest(to requested: Locale, in supported: [Locale]) -> Locale? {
        let candidates = supported.filter { $0.language.languageCode == requested.language.languageCode }
        let region = requested.region ?? Locale.Language(identifier: requested.language.maximalIdentifier).region
        return candidates.first { $0.region == region } ?? candidates.first
    }

    private static func install(locale: Locale, dictation: Bool) async -> SpeechListener? {
        let modules = [self.module(locale: locale, dictation: dictation)]
        do {
            if let request = try await AssetInventory.assetInstallationRequest(supporting: modules) {
                try await request.downloadAndInstall()
            }
        } catch {
            return nil
        }
        guard let format = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: modules) else { return nil }
        return SpeechListener(locale: locale, dictation: dictation, format: format)
    }

    private static func module(locale: Locale, dictation: Bool) -> any SpeechModule {
        if dictation {
            return DictationTranscriber(locale: locale, preset: .progressiveShortDictation)
        }
        return SpeechTranscriber(locale: locale, preset: .progressiveTranscription)
    }
}

nonisolated private enum SpeechFeedError: Error {
    case unsupportedFormat
}

nonisolated private final class SpeechFeed: @unchecked Sendable {
    private let converter: AVAudioConverter
    private let output: AVAudioFormat
    private let continuation: AsyncStream<AnalyzerInput>.Continuation

    init(
        from input: AVAudioFormat,
        to output: AVAudioFormat,
        into continuation: AsyncStream<AnalyzerInput>.Continuation
    ) throws {
        guard input.sampleRate > 0, input.channelCount > 0,
              let converter = AVAudioConverter(from: input, to: output)
        else {
            throw SpeechFeedError.unsupportedFormat
        }
        self.converter = converter
        self.output = output
        self.continuation = continuation
    }

    func append(_ buffer: AVAudioPCMBuffer) {
        let ratio = self.output.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount((Double(buffer.frameLength) * ratio).rounded(.up)) + 32
        guard let converted = AVAudioPCMBuffer(pcmFormat: self.output, frameCapacity: capacity) else { return }
        var supplied = false
        var error: NSError?
        let status = self.converter.convert(to: converted, error: &error) { _, inputStatus in
            guard !supplied else {
                inputStatus.pointee = .noDataNow
                return nil
            }
            supplied = true
            inputStatus.pointee = .haveData
            return buffer
        }
        guard status != .error, converted.frameLength > 0 else { return }
        self.continuation.yield(AnalyzerInput(buffer: converted))
    }

    func end() {
        self.continuation.finish()
    }

    static func level(of buffer: AVAudioPCMBuffer) -> Double {
        guard let samples = buffer.floatChannelData?[0], buffer.frameLength > 0 else { return 0 }
        let count = Int(buffer.frameLength)
        var sum: Float = 0
        for index in 0 ..< count {
            sum += samples[index] * samples[index]
        }
        let rms = sqrt(sum / Float(count))
        let decibels = 20 * log10(max(rms, 0.000_01))
        return Double(min(max((decibels + 50) / 45, 0), 1))
    }
}
