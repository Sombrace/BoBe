import Foundation

enum VoiceTtsBackend: Sendable, Equatable {
    case serverPocket
    case systemVoice

    static func best(for language: String) -> Self {
        PocketVoices.speaks(language) ? .serverPocket : .systemVoice
    }

    var engineId: String {
        switch self {
        case .serverPocket: VoiceEngineId.serverPocket
        case .systemVoice: VoiceEngineId.appleSpeechSynthesizer
        }
    }

    static func languageKey(_ code: String) -> String {
        String(code.lowercased().prefix(2))
    }
}

enum VoiceLanguageAdvice {
    static func pocketVoice(language: String, persona: String) -> String? {
        guard PocketVoices.speaks(language),
              PocketVoices.language(ofSlot: persona) != VoiceTtsBackend.languageKey(language)
        else { return nil }
        return PocketVoices.slot(for: language, like: persona)
    }
}
