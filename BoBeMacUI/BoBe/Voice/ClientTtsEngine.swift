@preconcurrency import AVFoundation
import FluidAudio
import Foundation

actor ClientTtsEngine {
    private let manager = Supertonic3Manager()
    private var style: Supertonic3VoiceStyle?
    private var styleVoice: Supertonic3Voice?
    private var initialized = false
    private var warmed = false

    func prepare() async throws {
        if !self.initialized {
            try await self.manager.initialize()
            self.initialized = true
        }
        if self.style == nil {
            self.style = try await Supertonic3ResourceDownloader.loadVoiceStyle(.f1)
            self.styleVoice = .f1
        }
        if !self.warmed, let style = self.style {
            _ = try await self.manager.synthesize(
                text: "Ready.",
                language: "en",
                style: style
            )
            self.warmed = true
        }
    }

    func synthesize(
        text: String,
        language: String,
        voiceId: String?,
        speed: Float
    ) async throws -> [Int16] {
        try Task.checkCancellation()
        try await self.prepare()
        let voice = Self.supertonicVoice(for: voiceId)
        let style: Supertonic3VoiceStyle
        if let cached = self.style, self.styleVoice == voice {
            style = cached
        } else {
            let loaded = try await Supertonic3ResourceDownloader.loadVoiceStyle(voice)
            self.style = loaded
            self.styleVoice = voice
            style = loaded
        }
        let result = try await self.manager.synthesize(
            text: text,
            language: Self.supertonicLanguage(for: language),
            style: style,
            speed: max(0.5, min(2.0, speed))
        )
        try Task.checkCancellation()
        return try Self.convertTo24kInt16(result.samples)
    }

    func cleanup() async {
        await self.manager.cleanup()
        self.style = nil
        self.styleVoice = nil
        self.initialized = false
        self.warmed = false
    }

    private static func supertonicVoice(for voiceId: String?) -> Supertonic3Voice {
        guard let voiceId else { return .f1 }
        let normalized = voiceId.lowercased()
        return normalized.hasPrefix("am_") || normalized.hasPrefix("bm_")
            ? .m1
            : .f1
    }

    private static func supertonicLanguage(for code: String) -> String {
        switch code.lowercased() {
        case "zh-cn", "zh-hans": "zh"
        case "pt-br": "pt"
        case "de-de": "de"
        case "es-es": "es"
        case "fr-fr": "fr"
        case "el-gr": "el"
        case "ko-kr": "ko"
        case "ja-jp": "ja"
        default: String(code.prefix(2)).lowercased()
        }
    }

    private static func convertTo24kInt16(_ samples: [Float]) throws -> [Int16] {
        guard let sourceFormat = AVAudioFormat(
            commonFormat: .pcmFormatFloat32,
            sampleRate: 44_100,
            channels: 1,
            interleaved: false
        ),
            let targetFormat = AVAudioFormat(
                commonFormat: .pcmFormatInt16,
                sampleRate: 24_000,
                channels: 1,
                interleaved: false
            ),
            let source = AVAudioPCMBuffer(
                pcmFormat: sourceFormat,
                frameCapacity: AVAudioFrameCount(samples.count)
            ),
            let sourceData = source.floatChannelData?[0],
            let converter = AVAudioConverter(from: sourceFormat, to: targetFormat)
        else {
            throw ClientTtsEngineError.audioConversionFailed
        }
        source.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { values in
            guard let base = values.baseAddress else { return }
            sourceData.update(from: base, count: values.count)
        }

        let capacity = AVAudioFrameCount(Double(samples.count) * 24_000 / 44_100) + 16
        guard let output = AVAudioPCMBuffer(pcmFormat: targetFormat, frameCapacity: capacity) else {
            throw ClientTtsEngineError.audioConversionFailed
        }
        let (status, error) = convertSingleBuffer(converter, source: source, into: output)
        guard status != .error, error == nil, let data = output.int16ChannelData?[0] else {
            throw ClientTtsEngineError.audioConversionFailed
        }
        return Array(UnsafeBufferPointer(start: data, count: Int(output.frameLength)))
    }
}

nonisolated enum ClientTtsEngineError: Error {
    case audioConversionFailed
}
