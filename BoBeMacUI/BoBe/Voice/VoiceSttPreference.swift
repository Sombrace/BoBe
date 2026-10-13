import Foundation
import Observation

enum VoiceSttPreference: String, CaseIterable, Sendable {
    case automatic
    case appleSpeech = "apple_speech"
    case fluidAudio = "fluid_audio"
    case daemon

    var placement: SpeechPlacementOverrideWire {
        self == .daemon ? .daemon : .auto
    }

    var usesEndpointEngine: Bool {
        self != .daemon
    }

    func speechRecognitionReady(
        endpointReady: Bool,
        daemonReady: Bool
    ) -> Bool {
        self == .daemon ? daemonReady : endpointReady || daemonReady
    }

    var label: String {
        L10n.tr("settings.voice.stt_preference.\(self.rawValue)")
    }
}

@MainActor
@Observable
final class VoiceSttPreferenceStore {
    static let shared = VoiceSttPreferenceStore()

    private static let key = "bobe.voice_stt_preference"

    private(set) var preference: VoiceSttPreference

    private init() {
        self.preference = UserDefaults.standard.string(forKey: Self.key)
            .flatMap(VoiceSttPreference.init(rawValue:)) ?? .automatic
    }

    func setPreference(_ preference: VoiceSttPreference) {
        self.preference = preference
        UserDefaults.standard.set(preference.rawValue, forKey: Self.key)
    }
}
