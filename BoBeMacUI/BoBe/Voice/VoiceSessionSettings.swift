import Foundation

struct VoiceSessionSettings: Equatable, Sendable {
    let language: String
    let persona: String?
    let speed: Float?
    let ttsBackend: VoiceTtsBackend
    let sttPreference: VoiceSttPreference
    let remoteAudioConsent: Bool

    func adoptingDaemonDefaults(from desired: Self) -> Self {
        Self(
            language: self.language,
            persona: self.persona ?? desired.persona,
            speed: self.speed ?? desired.speed,
            ttsBackend: self.ttsBackend,
            sttPreference: self.sttPreference,
            remoteAudioConsent: self.remoteAudioConsent
        )
    }

    static func current() -> Self {
        let settings = SettingsStore.shared.settings
        let consent = RemoteSpeechConsent.shared
        return Self(
            language: VoicePipeline.sttLanguage(from: settings),
            persona: settings?.voicePersona,
            speed: settings?.voiceSpeed,
            ttsBackend: VoiceTtsPreference.shared.backend,
            sttPreference: VoiceSttPreferenceStore.shared.preference,
            remoteAudioConsent: consent.isEnabled && consent.currentAuthorityEnabled
        )
    }
}
