import Foundation

struct VoiceSessionSettings: Equatable, Sendable {
    let language: String
    let persona: String?
    let speed: Float?
    let remoteAudioConsent: Bool

    var ttsBackend: VoiceTtsBackend {
        .best(for: self.language)
    }

    func adoptingDaemonDefaults(from desired: Self) -> Self {
        Self(
            language: self.language,
            persona: self.persona ?? desired.persona,
            speed: self.speed ?? desired.speed,
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
            remoteAudioConsent: consent.isEnabled && consent.currentAuthorityEnabled
        )
    }
}
