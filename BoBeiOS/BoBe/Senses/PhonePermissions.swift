import AVFoundation
import Speech

enum PhonePermission: Sendable, Equatable {
    case unknown
    case granted
    case denied
}

enum PhonePermissions {
    static func microphone() -> PhonePermission {
        switch AVAudioApplication.shared.recordPermission {
        case .granted: .granted
        case .denied: .denied
        case .undetermined: .unknown
        @unknown default: .unknown
        }
    }

    static func speech() -> PhonePermission {
        switch SFSpeechRecognizer.authorizationStatus() {
        case .authorized: .granted
        case .denied, .restricted: .denied
        case .notDetermined: .unknown
        @unknown default: .unknown
        }
    }

    static func voice() -> PhonePermission {
        let microphone = self.microphone()
        let speech = self.speech()
        if microphone == .granted, speech == .granted { return .granted }
        if microphone == .denied || speech == .denied { return .denied }
        return .unknown
    }

    static func requestVoice() async -> Bool {
        guard await AVAudioApplication.requestRecordPermission() else { return false }
        return await withCheckedContinuation { continuation in
            SFSpeechRecognizer.requestAuthorization { status in
                continuation.resume(returning: status == .authorized)
            }
        }
    }

    static func camera() -> PhonePermission {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized: .granted
        case .denied, .restricted: .denied
        case .notDetermined: .unknown
        @unknown default: .unknown
        }
    }

    static func requestCamera() async -> Bool {
        await AVCaptureDevice.requestAccess(for: .video)
    }
}
