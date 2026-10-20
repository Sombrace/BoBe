@preconcurrency import AVFoundation
@testable import BoBe
import Foundation
import Testing

@Suite(
    "Apple speech round trip",
    .serialized,
    .enabled(if: ProcessInfo.processInfo.environment["BOBE_RUN_MODEL_TESTS"] == "1")
)
struct AppleSpeechRoundTripTests {
    actor Transcript {
        private(set) var partials: [String] = []
        private(set) var finals: [String] = []
        private(set) var failures: [String] = []

        func partial(_ text: String) {
            self.partials.append(text)
        }

        func final(_ text: String) {
            self.finals.append(text)
        }

        func failure(_ message: String) {
            self.failures.append(message)
        }
    }

    @Test(arguments: [
        ("en", "Can you check the weather in Athens tomorrow morning.", "Athens"),
        ("el", "Τι καιρό θα κάνει αύριο στην Αθήνα;", "Αθήνα"),
    ])
    func systemVoiceIsHeardOnceAndEndsTheTurn(language: String, sentence: String, keyword: String) async throws {
        let voice = try #require(SystemVoiceChoice.best(for: language))
        let speech = try await SystemVoice().synthesize(text: sentence, voiceIdentifier: voice.identifier, speed: 1)
        let microphone = try await Self.microphoneSamples(speech)

        let transcript = Transcript()
        let stt = AppleSpeechStt(language: language, pauseMs: PauseSensitivityMs.balanced)
        try await stt.load(
            onPartial: { await transcript.partial($0) },
            onEou: { await transcript.final($0) },
            onFailure: { await transcript.failure($0) },
            onProgress: { _ in }
        )
        let started = ContinuousClock.now
        for chunk in microphone.chunks(ofCount: 640) {
            try await stt.acceptSamples(Array(chunk))
            try await Task.sleep(for: .milliseconds(10))
        }
        while await transcript.finals.isEmpty, started.duration(to: .now) < .seconds(20) {
            try await Task.sleep(for: .milliseconds(50))
        }
        await stt.cleanup()

        let finals = await transcript.finals
        #expect(await transcript.failures.isEmpty)
        #expect(finals.count == 1)
        let heard = try #require(finals.first)
        #expect(heard.localizedCaseInsensitiveContains(keyword), "\(heard)")
        let words = heard.split(whereSeparator: \.isWhitespace)
        #expect(Set(words).count * 2 > words.count, "\(heard)")
        #expect(await !transcript.partials.isEmpty)
    }

    private static func microphoneSamples(_ playback: [Int16]) async throws -> [Float] {
        let format = try #require(AVAudioFormat(
            commonFormat: .pcmFormatInt16,
            sampleRate: Double(VoiceWire.ttsOutputSampleRate),
            channels: 1,
            interleaved: false
        ))
        let silence = [Int16](repeating: 0, count: VoiceWire.ttsOutputSampleRate * 2)
        let samples = [Int16](repeating: 0, count: VoiceWire.ttsOutputSampleRate / 4) + playback + silence
        let buffer = try #require(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(samples.count)))
        buffer.frameLength = AVAudioFrameCount(samples.count)
        let channel = try #require(buffer.int16ChannelData?[0])
        for (index, sample) in samples.enumerated() {
            channel[index] = sample
        }
        let captured = try #require(CapturedVoiceAudio(buffer: buffer))
        return try await VoiceInputProcessor().process(captured).samples16k
    }
}

private extension Array {
    func chunks(ofCount size: Int) -> [ArraySlice<Element>] {
        stride(from: 0, to: self.count, by: size).map { self[$0 ..< Swift.min($0 + size, self.count)] }
    }
}
