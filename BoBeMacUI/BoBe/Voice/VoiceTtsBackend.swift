import Foundation
import Observation

enum VoiceTtsBackend: String, CaseIterable, Sendable {
    case serverKokoro = "server_kokoro"
    case clientSupertonic = "client_supertonic"

    var label: String {
        L10n.tr("settings.voice.tts_backend.\(self.rawValue)")
    }

    var shortName: String {
        switch self {
        case .serverKokoro: "Kokoro"
        case .clientSupertonic: "Supertonic"
        }
    }

    private var languages: Set<String> {
        switch self {
        case .serverKokoro:
            ["en", "es", "fr", "hi", "it", "ja", "pt", "zh"]
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
        if VoiceTtsBackend.serverKokoro.speaks(language) { return .serverKokoro }
        if VoiceTtsBackend.clientSupertonic.speaks(language) { return .clientSupertonic }
        return .serverKokoro
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
        guard backend == .serverKokoro,
              KokoroVoices.language(ofSlot: persona) != VoiceTtsBackend.languageKey(language),
              let slot = KokoroVoices.slot(for: language, like: persona)
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
            .flatMap(VoiceTtsBackend.init(rawValue:)) ?? .serverKokoro
    }

    func setBackend(_ backend: VoiceTtsBackend) {
        self.backend = backend
        UserDefaults.standard.set(backend.rawValue, forKey: Self.key)
    }
}
