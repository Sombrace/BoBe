@testable import BoBe
import Foundation
import Testing

@Suite("voiceWsEndpoint(from:)")
struct VoiceWsEndpointTests {
    @Test
    func httpBecomesWs() throws {
        let url = try #require(URL(string: "http://127.0.0.1:8766"))
        let ws = voiceWsEndpoint(from: url)
        #expect(ws?.absoluteString == "ws://127.0.0.1:8766/voice/stream")
    }

    @Suite("VoiceSttLoadProgress")
    struct VoiceSttLoadProgressTests {
        @Test(arguments: [(0.426, 43), (-1, 0), (2, 100)])
        func percentRoundsAndClamps(fraction: Double, percent: Int) {
            #expect(VoiceSttLoadProgress(fractionCompleted: fraction).percent == percent)
        }
    }

    @Suite("VoiceSampleBuffer")
    struct VoiceSampleBufferTests {
        @Test
        func preservesFramesAcrossBatchedCompaction() throws {
            var buffer = VoiceSampleBuffer()
            let samples = (0 ..< 9_000).map(Float.init)
            buffer.append(contentsOf: samples)

            for frameIndex in 0 ..< 25 {
                let frame = try #require(buffer.firstFrame(count: 320))
                #expect(frame.first == Float(frameIndex * 320))
                #expect(frame.last == Float(frameIndex * 320 + 319))
                let consumed = buffer.consumeFirst(320)
                #expect(consumed)
            }

            #expect(buffer.count == 1_000)
            let tail = try #require(buffer.firstFrame(count: 320))
            #expect(tail.first == 8_000)
            #expect(tail.last == 8_319)
        }

        @Test
        func doesNotAdvanceUntilExplicitRemoval() throws {
            var buffer = VoiceSampleBuffer()
            buffer.append(contentsOf: (0 ..< 640).map(Float.init))

            let first = try #require(buffer.firstFrame(count: 320))
            let repeated = try #require(buffer.firstFrame(count: 320))
            #expect(first.elementsEqual(repeated))
            #expect(buffer.count == 640)

            let consumed = buffer.consumeFirst(320)
            #expect(consumed)
            #expect(try #require(buffer.firstFrame(count: 320)).first == 320)
        }

        @Test
        func staleConsumptionAfterTeardownDoesNotTrap() {
            var buffer = VoiceSampleBuffer()
            buffer.append(contentsOf: (0 ..< 320).map(Float.init))
            buffer.removeAll()

            let consumed = buffer.consumeFirst(320)
            #expect(!consumed)
            #expect(buffer.count == 0)
        }

        @Test
        func removeAllResetsBufferedState() {
            var buffer = VoiceSampleBuffer()
            buffer.append(contentsOf: [1, 2, 3])
            buffer.removeAll()

            #expect(buffer.count == 0)
            #expect(buffer.firstFrame(count: 1) == nil)
        }
    }

    @Suite("Speech pause")
    struct SpeechPauseTests {
        private static let frame = SpeechPause.frameSamples

        private static func tone(frames: Int, amplitude: Float = 0.2) -> [Float] {
            (0 ..< frames * Self.frame).map { amplitude * sin(Float($0) * 0.2) }
        }

        private static func silence(frames: Int, level: Float = 0) -> [Float] {
            [Float](repeating: level, count: frames * Self.frame)
        }

        @Test
        func silenceAloneNeverEndsATurn() {
            var pause = SpeechPause()
            pause.observe(Self.silence(frames: 200))
            #expect(!pause.heardSpeech)
            #expect(!pause.reached(.milliseconds(400)))
        }

        @Test(arguments: [400, 800, 1_500])
        func aPauseAfterSpeechEndsTheTurnAtTheChosenSensitivity(_ milliseconds: Int) {
            var pause = SpeechPause()
            pause.observe(Self.tone(frames: 50))
            let frames = milliseconds / 20
            pause.observe(Self.silence(frames: frames - 1))
            #expect(!pause.reached(.milliseconds(milliseconds)))
            pause.observe(Self.silence(frames: 1))
            #expect(pause.reached(.milliseconds(milliseconds)))
        }

        @Test
        func speechResumingBeforeThePauseKeepsTheTurnOpen() {
            var pause = SpeechPause()
            pause.observe(Self.tone(frames: 20))
            pause.observe(Self.silence(frames: 30))
            pause.observe(Self.tone(frames: 5))
            pause.observe(Self.silence(frames: 30))
            #expect(!pause.reached(.milliseconds(800)))
        }

        @Test
        func steadyRoomNoiseCountsAsSilence() {
            var pause = SpeechPause()
            pause.observe(Self.tone(frames: 40, amplitude: 0.3))
            pause.observe(Self.tone(frames: 60, amplitude: 0.004))
            #expect(pause.reached(.milliseconds(800)))
        }

        @Test
        func partialFramesCarryOverBetweenChunks() {
            var pause = SpeechPause()
            let speech = Self.tone(frames: 4)
            pause.observe(Array(speech[..<(Self.frame / 2)]))
            #expect(!pause.heardSpeech)
            pause.observe(Array(speech[(Self.frame / 2)...]))
            #expect(pause.heardSpeech)
        }
    }

    @Suite("Speech recognition locale")
    struct SpeechRecognitionLocaleTests {
        private static let supported = ["en_GB", "en_US", "es_ES", "es_MX", "zh_CN", "zh_TW", "el_GR"].map(Locale.init)

        @Test(arguments: [
            ("en", "en_US"),
            ("en-GB", "en_GB"),
            ("es", "es_ES"),
            ("es-MX", "es_MX"),
            ("zh", "zh_CN"),
            ("el", "el_GR"),
        ])
        func choosesTheLanguagesHomeRegion(requested: String, expected: String) {
            let chosen = SpeechRecognitionModule.closest(to: Locale(identifier: requested), in: Self.supported)
            #expect(chosen?.identifier == expected)
        }

        @Test
        func anUnlistedLanguageHasNoRecognizer() {
            #expect(SpeechRecognitionModule.closest(to: Locale(identifier: "ko"), in: Self.supported) == nil)
        }
    }

    @Suite("Model preparation independent of voice policy")
    struct VoiceModelPreparationTests {
        @Test
        func deferringVoiceSurvivesPreferenceReload() throws {
            let suite = "VoiceSetupPreferenceTests.\(UUID().uuidString)"
            let defaults = try #require(UserDefaults(suiteName: suite))
            defer { defaults.removePersistentDomain(forName: suite) }
            VoiceSetupPreference.setDeferred(true, defaults: defaults)
            #expect(VoiceSetupPreference.isDeferred(in: defaults))
            VoiceSetupPreference.setDeferred(false, defaults: defaults)
            #expect(!VoiceSetupPreference.isDeferred(in: defaults))
        }

        @Test
        func finishedModelsAreReadyEvenWithStaleInstallerFailure() {
            #expect(VoiceModelPreparation.resolve(
                snapshotAvailable: true, ttsReady: true, sttReady: true,
                installing: false, failure: "old failure"
            ) == .ready)
        }

        @Test
        func completedDownloadsDoNotRequireVoicePermissionOrEnablement() {
            let preparation = VoiceModelPreparation.resolve(
                snapshotAvailable: true, ttsReady: true, sttReady: true,
                installing: false, failure: nil
            )
            #expect(preparation == .ready)
            #expect(preparation.resolveReadiness(voiceEnabled: false, permissionMissing: false) == .disabledByUser)
            #expect(preparation.resolveReadiness(voiceEnabled: true, permissionMissing: true) == .permissionMissing)
            #expect(preparation.resolveReadiness(voiceEnabled: true, permissionMissing: false) == .ready)
        }

        @Test
        func incompleteDownloadsRemainInstalling() {
            #expect(VoiceModelPreparation.resolve(
                snapshotAvailable: true, ttsReady: false, sttReady: true,
                installing: true, failure: nil
            ) == .installing)
        }

        @Test
        func canceledPreparationReturnsToMissing() {
            #expect(VoiceModelPreparation.resolve(
                snapshotAvailable: true, ttsReady: false, sttReady: true,
                installing: false, failure: nil
            ) == .missing)
        }

        @Test
        func currentFailureIsTerminal() {
            #expect(VoiceModelPreparation.resolve(
                snapshotAvailable: true, ttsReady: false, sttReady: true,
                installing: false, failure: "download failed"
            ) == .failed("download failed"))
        }
    }

    @Suite("Voice install outcome")
    struct VoiceInstallOutcomeTests {
        @Test
        func inFlightInstallIsNeverSettled() {
            #expect(VoiceInstallOutcome.resolve(
                installing: true, snapshotRunning: nil,
                preparation: .failed("old failure"), sttStatus: .failed("old failure")
            ) == .installing)
        }

        @Test
        func missingSnapshotMeansDaemonUnreachable() {
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: nil,
                preparation: .preparing, sttStatus: .notLoaded
            ) == .failed(L10n.tr("setup.voice.daemon_unreachable")))
        }

        @Test
        func readyModelsWinOverStaleRecognitionFailure() {
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: false,
                preparation: .ready, sttStatus: .failed("stale")
            ) == .ready)
        }

        @Test
        func recognitionFailureEndsInstallEvenWhileVoiceDownloads() {
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: true,
                preparation: .installing, sttStatus: .failed("hearing failed")
            ) == .failed("hearing failed"))
        }

        @Test
        func runningDownloadsKeepInstalling() {
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: true,
                preparation: .installing, sttStatus: .ready
            ) == .installing)
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: false,
                preparation: .installing, sttStatus: .downloading
            ) == .installing)
        }

        @Test
        func stoppedDownloadsReturnToTheChoice() {
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: false,
                preparation: .missing, sttStatus: .notLoaded
            ) == .stopped)
        }

        @Test
        func retryAfterFailedDownloadWaitsForStatusFetchedAfterTheRequest() {
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: false,
                preparation: .failed("download failed"), sttStatus: .ready,
                snapshotRevision: 4, staleThrough: 4
            ) == .installing)
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: true,
                preparation: .installing, sttStatus: .ready,
                snapshotRevision: 5, staleThrough: 4
            ) == .installing)
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: false,
                preparation: .failed("download failed again"), sttStatus: .ready,
                snapshotRevision: 6, staleThrough: 4
            ) == .failed("download failed again"))
        }

        @Test
        func firstInstallIgnoresIdleStatusFromBeforeTheRequest() {
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: false,
                preparation: .missing, sttStatus: .ready,
                snapshotRevision: 1, staleThrough: 1
            ) == .installing)
        }

        @Test
        func preparationFailureIsReported() {
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: false,
                preparation: .failed("voice failed"), sttStatus: .ready
            ) == .failed("voice failed"))
        }
    }

    @Test
    func httpsBecomesWss() throws {
        let url = try #require(URL(string: "https://example.test:8443"))
        let ws = voiceWsEndpoint(from: url)
        #expect(ws?.absoluteString == "wss://example.test:8443/voice/stream")
    }

    @Test
    func preservesPathPrefix() throws {
        let url = try #require(URL(string: "https://example.test/bobe/api"))
        let ws = voiceWsEndpoint(from: url)
        #expect(ws?.absoluteString == "wss://example.test/bobe/api/voice/stream")
    }

    @Test
    func avoidsDoubleSlashAfterTrailingPrefixSlash() throws {
        let url = try #require(URL(string: "https://example.test/bobe/"))
        let ws = voiceWsEndpoint(from: url)
        #expect(ws?.path == "/bobe/voice/stream")
    }

    @Test
    func unknownSchemeFallsBackToWs() throws {
        let url = try #require(URL(string: "weird://host:80"))
        let ws = voiceWsEndpoint(from: url)
        #expect(ws?.scheme == "ws")
    }
}
