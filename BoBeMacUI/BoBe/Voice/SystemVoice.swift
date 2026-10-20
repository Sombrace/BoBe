@preconcurrency import AVFoundation
import Foundation
import Synchronization

nonisolated struct SystemVoiceChoice: Sendable, Equatable {
    let identifier: String
    let name: String
    let quality: AVSpeechSynthesisVoiceQuality

    static func best(for language: String) -> Self? {
        let requested = Locale(identifier: language)
        let region = requested.region ?? Locale.Language(identifier: requested.language.maximalIdentifier).region
        let voices = AVSpeechSynthesisVoice.speechVoices().filter { voice in
            Locale(identifier: voice.language).language.languageCode == requested.language.languageCode
                && !voice.voiceTraits.contains(.isNoveltyVoice)
                && !voice.voiceTraits.contains(.isPersonalVoice)
        }
        let best = voices.max { lhs, rhs in
            self.rank(lhs, region: region) < self.rank(rhs, region: region)
        }
        return best.map { Self(identifier: $0.identifier, name: $0.name, quality: $0.quality) }
    }

    private static func rank(_ voice: AVSpeechSynthesisVoice, region: Locale.Region?) -> Int {
        voice.quality.rawValue * 4
            + (voice.identifier.hasPrefix("com.apple.voice.") ? 2 : 0)
            + (Locale(identifier: voice.language).region == region ? 1 : 0)
    }
}

actor SystemVoice {
    private let synthesizer = AVSpeechSynthesizer()

    func synthesize(text: String, voiceIdentifier: String, speed: Float) async throws -> [Int16] {
        try Task.checkCancellation()
        guard let voice = AVSpeechSynthesisVoice(identifier: voiceIdentifier) else {
            throw SystemVoiceError.voiceUnavailable(voiceIdentifier)
        }
        let utterance = AVSpeechUtterance(string: text)
        utterance.voice = voice
        utterance.rate = min(
            AVSpeechUtteranceMaximumSpeechRate,
            max(AVSpeechUtteranceMinimumSpeechRate, AVSpeechUtteranceDefaultSpeechRate * max(0.5, min(2, speed)))
        )
        let capture = SpeechCapture()
        try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, any Error>) in
                capture.begin(continuation)
                Self.write(utterance, with: self.synthesizer, into: capture)
            }
        } onCancel: {
            capture.cancel()
            Task { await self.stop() }
        }
        let (samples, sampleRate) = capture.samples
        guard !samples.isEmpty else { throw SystemVoiceError.noAudio }
        return try Self.playbackSamples(samples, sampleRate: sampleRate)
    }

    private func stop() {
        self.synthesizer.stopSpeaking(at: .immediate)
    }

    nonisolated private static func write(
        _ utterance: AVSpeechUtterance,
        with synthesizer: AVSpeechSynthesizer,
        into capture: SpeechCapture
    ) {
        synthesizer.write(utterance) { @Sendable buffer in
            capture.append(buffer)
        }
    }

    nonisolated private static func playbackSamples(_ samples: [Float], sampleRate: Double) throws -> [Int16] {
        let targetRate = Double(VoiceWire.ttsOutputSampleRate)
        guard let sourceFormat = AVAudioFormat(
            commonFormat: .pcmFormatFloat32,
            sampleRate: sampleRate,
            channels: 1,
            interleaved: false
        ),
            let targetFormat = AVAudioFormat(
                commonFormat: .pcmFormatInt16,
                sampleRate: targetRate,
                channels: 1,
                interleaved: false
            ),
            let source = AVAudioPCMBuffer(pcmFormat: sourceFormat, frameCapacity: AVAudioFrameCount(samples.count)),
            let sourceData = source.floatChannelData?[0],
            let converter = AVAudioConverter(from: sourceFormat, to: targetFormat),
            let output = AVAudioPCMBuffer(
                pcmFormat: targetFormat,
                frameCapacity: AVAudioFrameCount(Double(samples.count) * targetRate / sampleRate) + 16
            )
        else {
            throw SystemVoiceError.audioConversionFailed
        }
        source.frameLength = AVAudioFrameCount(samples.count)
        samples.withUnsafeBufferPointer { values in
            guard let base = values.baseAddress else { return }
            sourceData.update(from: base, count: values.count)
        }
        let (status, error) = convertSingleBuffer(converter, source: source, into: output)
        guard status != .error, error == nil, let data = output.int16ChannelData?[0] else {
            throw SystemVoiceError.audioConversionFailed
        }
        return Array(UnsafeBufferPointer(start: data, count: Int(output.frameLength)))
    }
}

nonisolated private final class SpeechCapture: Sendable {
    private struct State {
        var samples: [Float] = []
        var sampleRate: Double = 0
        var continuation: CheckedContinuation<Void, any Error>?
        var finished = false
    }

    private let state = Mutex(State())

    var samples: ([Float], Double) {
        self.state.withLock { ($0.samples, $0.sampleRate) }
    }

    func begin(_ continuation: CheckedContinuation<Void, any Error>) {
        let cancelled = self.state.withLock { state in
            guard !state.finished else { return true }
            state.continuation = continuation
            return false
        }
        if cancelled {
            continuation.resume(throwing: CancellationError())
        }
    }

    func append(_ buffer: AVAudioBuffer) {
        guard let pcm = buffer as? AVAudioPCMBuffer else { return }
        let frames = Int(pcm.frameLength)
        let continuation = self.state.withLock { state -> CheckedContinuation<Void, any Error>? in
            guard !state.finished else { return nil }
            guard frames > 0 else {
                state.finished = true
                defer { state.continuation = nil }
                return state.continuation
            }
            state.sampleRate = pcm.format.sampleRate
            if let floats = pcm.floatChannelData?[0] {
                state.samples.append(contentsOf: UnsafeBufferPointer(start: floats, count: frames))
            } else if let integers = pcm.int16ChannelData?[0] {
                state.samples.append(contentsOf: UnsafeBufferPointer(start: integers, count: frames).lazy.map {
                    Float($0) / 32_768
                })
            }
            return nil
        }
        continuation?.resume()
    }

    func cancel() {
        let continuation = self.state.withLock { state -> CheckedContinuation<Void, any Error>? in
            guard !state.finished else { return nil }
            state.finished = true
            defer { state.continuation = nil }
            return state.continuation
        }
        continuation?.resume(throwing: CancellationError())
    }
}

nonisolated enum SystemVoiceError: LocalizedError {
    case voiceUnavailable(String)
    case noAudio
    case audioConversionFailed

    var errorDescription: String? {
        switch self {
        case let .voiceUnavailable(identifier):
            "The system voice \(identifier) is not installed."
        case .noAudio:
            "The system voice produced no audio."
        case .audioConversionFailed:
            "System voice audio could not be converted for playback."
        }
    }
}
