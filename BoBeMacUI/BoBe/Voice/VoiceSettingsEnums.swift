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

    static func displayName(for code: String, in locale: Locale = L10n.locale) -> String {
        locale.localizedString(forLanguageCode: code) ?? code
    }

    static func menuTitle(for code: String, in locale: Locale = L10n.locale) -> String {
        guard let name = locale.localizedString(forLanguageCode: code) else { return code }
        return name.prefix(1).uppercased(with: locale) + name.dropFirst()
    }
}
