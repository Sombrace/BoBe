import Foundation
import Observation

enum VoiceTtsBackend: String, CaseIterable, Sendable {
    case serverPocket = "server_pocket"
    case clientSupertonic = "client_supertonic"

    var label: String {
        L10n.tr("settings.voice.tts_backend.\(self.rawValue)")
    }

    var shortName: String {
        switch self {
        case .serverPocket: "Pocket TTS"
        case .clientSupertonic: "Supertonic"
        }
    }

    private var languages: Set<String> {
        switch self {
        case .serverPocket:
            ["en"]
        case .clientSupertonic:
            [
                "ar", "bg", "cs", "da", "de", "el", "en", "es", "et", "fi", "fr", "hi", "hr", "hu", "id", "it",
                "ja", "ko", "lt", "lv", "nl", "pl", "pt", "ro", "ru", "sk", "sl", "sv", "tr", "uk", "vi",
            ]
        }
    }

    func speaks(_ language: String) -> Bool {
        self.languages.contains(Self.languageKey(language))
    }

    static func best(for language: String) -> VoiceTtsBackend {
        if VoiceTtsBackend.serverPocket.speaks(language) { return .serverPocket }
        if VoiceTtsBackend.clientSupertonic.speaks(language) { return .clientSupertonic }
        return .serverPocket
    }

    static func languageKey(_ code: String) -> String {
        String(code.lowercased().prefix(2))
    }
}

enum VoiceLanguageAdvice: Equatable, Sendable {
    case engine(VoiceTtsBackend)
    case persona(String)

    static func advice(language: String, backend: VoiceTtsBackend, persona: String) -> VoiceLanguageAdvice? {
        let best = VoiceTtsBackend.best(for: language)
        if !backend.speaks(language) {
            return best.speaks(language) && best != backend ? .engine(best) : nil
        }
        guard backend == .serverPocket,
              PocketVoices.language(ofSlot: persona) != VoiceTtsBackend.languageKey(language),
              let slot = PocketVoices.slot(for: language, like: persona)
        else { return nil }
        return .persona(slot)
    }
}

@MainActor
@Observable
final class VoiceTtsPreference {
    static let shared = VoiceTtsPreference()

    private static let key = "bobe.voice_tts_backend"

    private(set) var backend: VoiceTtsBackend

    private init() {
        self.backend = UserDefaults.standard.string(forKey: Self.key)
            .flatMap(VoiceTtsBackend.init(rawValue:)) ?? .serverPocket
    }

    func setBackend(_ backend: VoiceTtsBackend) {
        self.backend = backend
        UserDefaults.standard.set(backend.rawValue, forKey: Self.key)
    }
}
