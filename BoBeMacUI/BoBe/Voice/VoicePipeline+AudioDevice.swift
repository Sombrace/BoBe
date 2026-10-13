@preconcurrency import AudioToolbox
import Foundation
import OSLog

private let logger = Logger(subsystem: "com.bobe.app", category: "VoicePipeline.AudioDevice")

@MainActor
extension VoicePipeline {

    func registerInputDeviceObserverIfNeeded() {
        self.registerDeviceListener(
            selector: kAudioHardwarePropertyDefaultInputDevice,
            kind: .input,
            alreadyRegistered: self.inputDeviceListenerRegistered,
            markRegistered: { self.inputDeviceListenerRegistered = true }
        )
        self.registerDeviceListener(
            selector: kAudioHardwarePropertyDefaultOutputDevice,
            kind: .output,
            alreadyRegistered: self.outputDeviceListenerRegistered,
            markRegistered: { self.outputDeviceListenerRegistered = true }
        )
    }

    private enum DeviceKind: String {
        case input, output
    }

    private func registerDeviceListener(
        selector: AudioObjectPropertySelector,
        kind: DeviceKind,
        alreadyRegistered: Bool,
        markRegistered: @escaping @MainActor () -> Void
    ) {
        guard !alreadyRegistered else { return }
        var address = AudioObjectPropertyAddress(
            mSelector: selector,
            mScope: kAudioObjectPropertyScopeGlobal,
            mElement: kAudioObjectPropertyElementMain
        )
        let status = AudioObjectAddPropertyListenerBlock(
            AudioObjectID(kAudioObjectSystemObject),
            &address,
            DispatchQueue.global(qos: .userInitiated)
        ) { @Sendable [weak self] _, _ in
            Task { @MainActor in
                self?.handleDefaultDeviceChange(kind: kind)
            }
        }
        if status == noErr {
            markRegistered()
            logger.info("voice: default-\(kind.rawValue, privacy: .public) device listener registered")
        } else {
            logger.warning(
                "voice: \(kind.rawValue, privacy: .public) device listener registration failed (status=\(status))"
            )
        }
    }


    private func handleDefaultDeviceChange(kind: DeviceKind) {
        logger.info("voice: default-\(kind.rawValue, privacy: .public) device changed")
        if self.audioRebuildMustWait {
            logger.info(
                "voice: deferring \(kind.rawValue, privacy: .public)-device rebuild — response active"
            )
            self.pendingAudioDeviceRebuild = true
        }
        self.scheduleDeviceChangeRebuild()
    }

    func handleAudioEngineConfigurationChange() {
        guard self.isWarm else { return }
        guard !self.audioEngine.isRunning else {
            logger.debug("voice: audio-engine configuration changed while running")
            return
        }
        logger.info("voice: audio-engine configuration changed; scheduling rebuild")
        self.pendingAudioDeviceRebuild = true
        self.scheduleDeviceChangeRebuild()
    }

    func drainPendingAudioDeviceRebuild() {
        guard self.pendingAudioDeviceRebuild else { return }
        logger.info("voice: response ended; reevaluating deferred audio-device rebuild")
        self.scheduleDeviceChangeRebuild()
    }

    private func scheduleDeviceChangeRebuild() {
        self.deviceChangeReconfigureTask?.cancel()
        self.deviceChangeReconfigureTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(250))
            guard let self, !Task.isCancelled else { return }
            while self.audioRebuildMustWait, !Task.isCancelled {
                self.pendingAudioDeviceRebuild = true
                try? await Task.sleep(for: .milliseconds(100))
            }
            guard !Task.isCancelled else { return }
            self.pendingAudioDeviceRebuild = false
            self.rebuildAudioEngineForDeviceChange()
            self.deviceChangeReconfigureTask = nil
        }
    }

    private var audioRebuildMustWait: Bool {
        self.state == .capturing
            || self.state == .thinking
            || self.state == .speaking
            || self.state == .cancelling
            || self.clientTtsActive
            || self.audioStillPlaying()
    }

    private func rebuildAudioEngineForDeviceChange() {
        guard self.isWarm else {
            logger.debug("voice: device change while cold — skipping rebuild")
            return
        }
        logger.info("voice: rebuilding audio engine for new default device")
        self.tearDownAudio()
        do {
            try self.configureEngine()
            self.isWarm = true
        } catch {
            self.lastError = "voice device-change rebuild: \(error.localizedDescription)"
            logger.error(
                "voice: engine rebuild after device change failed: \(error.localizedDescription, privacy: .public)"
            )
        }
    }
}
