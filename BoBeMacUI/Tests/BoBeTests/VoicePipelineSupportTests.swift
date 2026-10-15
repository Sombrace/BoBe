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
        @Test
        func percentRoundsAndClamps() {
            #expect(
                VoiceSttLoadProgress(
                    fractionCompleted: 0.426,
                    phase: .downloading(completedFiles: 2, totalFiles: 5)
                ).percent == 43
            )
            #expect(
                VoiceSttLoadProgress(
                    fractionCompleted: -1,
                    phase: .listing
                ).percent == 0
            )
            #expect(
                VoiceSttLoadProgress(
                    fractionCompleted: 2,
                    phase: .compiling(modelName: "encoder")
                ).percent == 100
            )
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

    @Suite("Apple Speech transcript normalization")
    struct AppleSpeechTranscriptTests {
        @Test(arguments: [
            ("", "hello", "hello"),
            ("hello", "", "hello"),
            ("hello", "hello world", "hello world"),
            ("hello world", "world", "hello world"),
            ("hello", "world", "hello world"),
            ("  hello  ", "  world  ", "hello world"),
        ])
        func mergesCommittedAndVolatileText(
            committed: String,
            candidate: String,
            expected: String
        ) {
            #expect(mergeSpeechTranscript(committed: committed, candidate: candidate) == expected)
        }
    }

    @Suite("Endpoint STT preference")
    struct EndpointSttPreferenceTests {
        @Test
        func daemonPreferenceForcesDaemonPlacement() {
            #expect(VoiceSttPreference.daemon.placement == .daemon)
        }

        @Test(arguments: [
            VoiceSttPreference.automatic,
            VoiceSttPreference.appleSpeech,
            VoiceSttPreference.fluidAudio,
        ])
        func endpointPreferencesPreserveDaemonAuthority(_ preference: VoiceSttPreference) {
            #expect(preference.placement == .auto)
            #expect(preference.usesEndpointEngine)
        }

        @Test
        func daemonPreferenceRequiresDaemonReadiness() {
            #expect(!VoiceSttPreference.daemon.usesEndpointEngine)
            #expect(!VoiceSttPreference.daemon.speechRecognitionReady(
                endpointReady: true,
                daemonReady: false
            ))
            #expect(VoiceSttPreference.daemon.speechRecognitionReady(
                endpointReady: false,
                daemonReady: true
            ))
        }

        @Test(arguments: [
            VoiceSttPreference.automatic,
            VoiceSttPreference.appleSpeech,
            VoiceSttPreference.fluidAudio,
        ])
        func endpointPreferencesCanFallBackToDaemon(_ preference: VoiceSttPreference) {
            #expect(preference.speechRecognitionReady(
                endpointReady: false,
                daemonReady: true
            ))
            #expect(!preference.speechRecognitionReady(
                endpointReady: false,
                daemonReady: false
            ))
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
        @MainActor
        func inFlightInstallIsNeverSettled() {
            #expect(VoiceInstallOutcome.resolve(
                installing: true, snapshotRunning: nil,
                preparation: .failed("old failure"), sttStatus: .failed("old failure")
            ) == .installing)
        }

        @Test
        @MainActor
        func missingSnapshotMeansDaemonUnreachable() {
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: nil,
                preparation: .preparing, sttStatus: .notLoaded
            ) == .failed(L10n.tr("setup.voice.daemon_unreachable")))
        }

        @Test
        @MainActor
        func readyModelsWinOverStaleRecognitionFailure() {
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: false,
                preparation: .ready, sttStatus: .failed("stale")
            ) == .ready)
        }

        @Test
        @MainActor
        func recognitionFailureEndsInstallEvenWhileVoiceDownloads() {
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: true,
                preparation: .installing, sttStatus: .failed("hearing failed")
            ) == .failed("hearing failed"))
        }

        @Test
        @MainActor
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
        @MainActor
        func stoppedDownloadsReturnToTheChoice() {
            #expect(VoiceInstallOutcome.resolve(
                installing: false, snapshotRunning: false,
                preparation: .missing, sttStatus: .notLoaded
            ) == .stopped)
        }

        @Test
        @MainActor
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
