@testable import BoBe
import Foundation
import Testing

@Suite("Voice session phases")
struct VoiceSessionPhaseTests {
    @Test
    func abortThenSpeakAgainKeepsTheSessionListening() throws {
        var state = VoicePipeline.State.speaking
        #expect(state.hasSession)
        state = .cancelling
        #expect(state.acceptsMicrophoneInput)
        state = try #require(VoicePipeline.State(serverPhase: .listening))
        #expect(state == .listening)
        #expect(state.acceptsMicrophoneInput)
        #expect(state.hasSession)
    }

    @Test
    func idleFromTheDaemonMeansTheSessionClosed() {
        #expect(VoicePipeline.State(serverPhase: .idle) == nil)
        #expect(!VoicePipeline.State.idle.hasSession)
        #expect(!VoicePipeline.State.failed("closed").hasSession)
        #expect(!VoicePipeline.State.idle.acceptsMicrophoneInput)
        #expect(!VoicePipeline.State.connecting.acceptsMicrophoneInput)
    }
}

@Suite("Reply playback receipts")
struct ReplyPlaybackTests {
    @Test
    func zeroRenderedSegmentsIsNeverHeard() {
        let playback = ReplyPlayback(turnId: "voice_a")
        #expect(playback.outcome(drained: true) == .unheard)
    }

    @Test
    func failureAfterPartialPlaybackIsNotHeard() {
        var playback = ReplyPlayback(turnId: "voice_a")
        playback.noteRendered()
        playback.noteRendered()
        playback.noteFailure()
        #expect(playback.failed)
        #expect(playback.outcome(drained: true) == .unheard)
    }

    @Test
    func playbackThatDidNotDrainIsNotHeard() {
        var playback = ReplyPlayback(turnId: "voice_a")
        playback.noteRendered()
        #expect(playback.outcome(drained: false) == .unheard)
    }

    @Test
    func everySegmentRenderedAndDrainedIsHeard() {
        var playback = ReplyPlayback(turnId: "voice_a")
        playback.noteRendered()
        playback.noteRendered()
        #expect(playback.renderedSegments == 2)
        #expect(playback.outcome(drained: true) == .heard)
    }
}

@Suite("Voice session settings")
struct VoiceSessionSettingsTests {
    private let negotiated = VoiceSessionSettings(
        language: "en",
        persona: "alba",
        speed: 1,
        ttsBackend: .serverPocket,
        sttPreference: .automatic,
        remoteAudioConsent: false
    )

    private func changing(
        language: String? = nil,
        persona: String? = nil,
        speed: Float? = nil,
        ttsBackend: VoiceTtsBackend? = nil,
        sttPreference: VoiceSttPreference? = nil,
        remoteAudioConsent: Bool? = nil
    ) -> VoiceSessionSettings {
        VoiceSessionSettings(
            language: language ?? self.negotiated.language,
            persona: persona ?? self.negotiated.persona,
            speed: speed ?? self.negotiated.speed,
            ttsBackend: ttsBackend ?? self.negotiated.ttsBackend,
            sttPreference: sttPreference ?? self.negotiated.sttPreference,
            remoteAudioConsent: remoteAudioConsent ?? self.negotiated.remoteAudioConsent
        )
    }

    @Test
    func everyNegotiatedSettingRequiresANewHello() {
        let changes = [
            self.changing(language: "es"),
            self.changing(persona: "marius"),
            self.changing(speed: 1.25),
            self.changing(ttsBackend: .clientSupertonic),
            self.changing(sttPreference: .fluidAudio),
            self.changing(remoteAudioConsent: true),
        ]
        for desired in changes {
            #expect(self.negotiated.adoptingDaemonDefaults(from: desired) != desired)
        }
        #expect(self.negotiated.adoptingDaemonDefaults(from: self.negotiated) == self.negotiated)
    }

    @Test
    func daemonDefaultsLearnedAfterHelloDoNotRestartTheSession() {
        let beforeSettingsLoaded = VoiceSessionSettings(
            language: "en",
            persona: nil,
            speed: nil,
            ttsBackend: .serverPocket,
            sttPreference: .automatic,
            remoteAudioConsent: false
        )
        let adopted = beforeSettingsLoaded.adoptingDaemonDefaults(from: self.negotiated)
        #expect(adopted == self.negotiated)
        #expect(adopted.adoptingDaemonDefaults(from: self.changing(persona: "javert")) != self.changing(persona: "javert"))
    }
}

@MainActor
private final class LifecycleLog {
    var events: [String] = []
}

@Suite("Audio lifecycle")
@MainActor
struct AudioLifecycleTests {
    @Test
    func configurationWaitsForEarlierTeardownCleanup() async {
        let lifecycle = AudioLifecycle()
        let log = LifecycleLog()
        lifecycle.invalidate()
        lifecycle.enqueue {
            try? await Task.sleep(for: .milliseconds(30))
            log.events.append("input stopped")
        }
        lifecycle.enqueue {
            log.events.append("recognizer reset")
        }
        await lifecycle.settled()
        log.events.append("capture started")
        #expect(log.events == ["input stopped", "recognizer reset", "capture started"])
    }

    @Test
    func teardownDuringConfigurationIsDetected() async {
        let lifecycle = AudioLifecycle()
        lifecycle.enqueue {
            try? await Task.sleep(for: .milliseconds(30))
        }
        let generation = lifecycle.generation
        let waiting = Task { @MainActor in
            await lifecycle.settled()
            return lifecycle.generation == generation
        }
        lifecycle.invalidate()
        lifecycle.enqueue {}
        #expect(await waiting.value == false)
    }
}
