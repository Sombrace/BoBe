@testable import BoBe
import Foundation
import Testing

@Suite("Voice abort replies")
struct VoiceAbortReplyTests {
    private static let sessionId = "voice-1700000000"

    private static func state(_ phase: String, _ turnId: String) -> String {
        #"{"type":"state","phase":"\#(phase)","turn_id":"\#(turnId)"}"#
    }

    private static func truncate(_ turnId: String) -> String {
        #"{"type":"truncate","turn_id":"\#(turnId)","keep_ms":0}"#
    }

    private static func receive(
        _ reply: [String],
        after state: VoicePipeline.State,
        _ tracker: inout VoicePhaseTracker
    ) throws -> VoicePipeline.State? {
        var current: VoicePipeline.State? = state
        for line in reply {
            let message = try JSONDecoder().decode(ServerVoiceMessage.self, from: Data(line.utf8))
            guard case let .state(phase, turnId) = message else { continue }
            current = tracker.state(after: phase, turnId: turnId, sessionId: Self.sessionId)
            guard current != nil else { return nil }
        }
        return current
    }

    @Test
    func a090DaemonAnswersStopWithIdleAndVoiceKeepsListening() throws {
        var tracker = VoicePhaseTracker()
        tracker.abortRequested()
        let reply = [Self.truncate("voice_turn"), Self.state("listening", "voice_turn"), Self.state("idle", Self.sessionId)]
        let received = try Self.receive(reply, after: .cancelling, &tracker)
        let state = try #require(received)
        #expect(state == .listening)
        #expect(state.acceptsMicrophoneInput)
        #expect(tracker.unacknowledgedAborts == 0)
    }

    @Test
    func a090DaemonAnswersStopWithNoTurnInFlightWithIdleAlone() throws {
        var tracker = VoicePhaseTracker()
        tracker.abortRequested()
        let state = try Self.receive([Self.state("idle", Self.sessionId)], after: .cancelling, &tracker)
        #expect(state == .listening)
        #expect(tracker.unacknowledgedAborts == 0)
    }

    @Test
    func a091DaemonAnswersStopWithListeningAndALaterIdleClosesTheSession() throws {
        var tracker = VoicePhaseTracker()
        tracker.abortRequested()
        let reply = [Self.truncate("voice_turn"), Self.state("listening", "voice_turn"), Self.state("listening", Self.sessionId)]
        let acknowledged = try Self.receive(reply, after: .cancelling, &tracker)
        #expect(acknowledged == .listening)
        #expect(tracker.unacknowledgedAborts == 0)
        let closed = try Self.receive([Self.state("idle", Self.sessionId)], after: .listening, &tracker)
        #expect(closed == nil)
    }

    @Test
    func a091DaemonAnswersStopWithNoTurnInFlightWithSessionListening() throws {
        var tracker = VoicePhaseTracker()
        tracker.abortRequested()
        let acknowledged = try Self.receive([Self.state("listening", Self.sessionId)], after: .cancelling, &tracker)
        #expect(acknowledged == .listening)
        let closed = try Self.receive([Self.state("idle", Self.sessionId)], after: .listening, &tracker)
        #expect(closed == nil)
    }

    @Test
    func anUnsolicitedIdleClosesTheSession() throws {
        var tracker = VoicePhaseTracker()
        let state = try Self.receive(
            [Self.state("listening", "voice_turn"), Self.state("idle", Self.sessionId)],
            after: .speaking,
            &tracker
        )
        #expect(state == nil)
        #expect(!VoicePipeline.State.idle.hasSession)
    }

    @Test
    func aTurnPhaseAlreadyInFlightDoesNotUseUpTheStopAcknowledgement() throws {
        var tracker = VoicePhaseTracker()
        tracker.abortRequested()
        let reply = [
            Self.state("thinking", "voice_next"),
            Self.truncate("voice_next"),
            Self.state("listening", "voice_next"),
            Self.state("idle", Self.sessionId),
        ]
        let state = try Self.receive(reply, after: .cancelling, &tracker)
        #expect(state == .listening)
    }

    @Test
    func everyStopIsAcknowledgedOnce() throws {
        var tracker = VoicePhaseTracker()
        tracker.abortRequested()
        tracker.abortRequested()
        let reply = [
            Self.truncate("voice_turn"),
            Self.state("listening", "voice_turn"),
            Self.state("idle", Self.sessionId),
            Self.state("idle", Self.sessionId),
        ]
        let acknowledged = try Self.receive(reply, after: .cancelling, &tracker)
        #expect(acknowledged == .listening)
        let closed = try Self.receive([Self.state("idle", Self.sessionId)], after: .listening, &tracker)
        #expect(closed == nil)
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
