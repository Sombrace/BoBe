import Foundation

enum PocketVoices {
    static let allSlots = ["alba", "azelma", "eponine", "fantine", "javert", "marius"]

    static func displayName(for slot: String) -> String {
        slot.capitalized
    }

    static func language(ofSlot slot: String) -> String? {
        self.allSlots.contains(slot) ? "en" : nil
    }

    static func slot(for language: String, like current: String) -> String? {
        guard VoiceTtsBackend.languageKey(language) == "en" else { return nil }
        return self.allSlots.contains(current) ? current : VoiceWire.defaultPersona
    }
}

enum VoiceLanguages {
    static let all: [String] = ["en", "zh", "es", "el", "ko", "ja"]

    static func displayName(for code: String) -> String {
        let base: String = switch code {
        case "en": "English"
        case "zh": "Mandarin"
        case "es": "Spanish"
        case "el": "Greek"
        case "ko": "Korean"
        case "ja": "Japanese"
        default: code
        }
        return base
    }
}
