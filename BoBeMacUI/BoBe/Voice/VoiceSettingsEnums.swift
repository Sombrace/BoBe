import Foundation

enum KokoroVoices {
    static let allSlots: [String] = [
        "af_alloy", "af_aoede", "af_bella", "af_heart", "af_jessica",
        "af_kore", "af_nicole", "af_nova", "af_river", "af_sarah", "af_sky",
        "am_adam", "am_echo", "am_eric", "am_fenrir", "am_liam",
        "am_michael", "am_onyx", "am_puck", "am_santa",
        "bf_alice", "bf_emma", "bf_isabella", "bf_lily",
        "bm_daniel", "bm_fable", "bm_george", "bm_lewis",
        "ef_dora", "em_alex",
        "ff_siwis",
        "hf_alpha", "hf_beta", "hm_omega", "hm_psi",
        "if_sara", "im_nicola",
        "jf_alpha", "jf_gongitsune", "jf_nezumi", "jf_tebukuro", "jm_kumo",
        "pf_dora", "pm_alex", "pm_santa",
        "zf_xiaobei", "zf_xiaoni", "zf_xiaoxiao", "zf_xiaoyi",
        "zm_yunjian", "zm_yunxi", "zm_yunxia", "zm_yunyang",
    ]

    static func displayName(for slot: String) -> String {
        let parts = slot.split(separator: "_", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return slot }
        let prefix = parts[0]
        let name = parts[1].capitalized
        let lang = self.languageLabel(forPrefix: prefix)
        return "\(name) (\(lang))"
    }

    static func language(ofSlot slot: String) -> String? {
        switch slot.first {
        case "a", "b": "en"
        case "e": "es"
        case "f": "fr"
        case "h": "hi"
        case "i": "it"
        case "j": "ja"
        case "p": "pt"
        case "z": "zh"
        default: nil
        }
    }

    static func slot(for language: String, like current: String) -> String? {
        let key = VoiceTtsBackend.languageKey(language)
        let matches = self.allSlots.filter { self.language(ofSlot: $0) == key }
        let gender = current.dropFirst().first
        return matches.first { $0.dropFirst().first == gender } ?? matches.first
    }

    private static func languageLabel(forPrefix prefix: String) -> String {
        let lang = switch prefix.first {
        case "a": "English"
        case "b": "British English"
        case "e": "Spanish"
        case "f": "French"
        case "h": "Hindi"
        case "i": "Italian"
        case "j": "Japanese"
        case "p": "Portuguese"
        case "z": "Mandarin"
        default: "?"
        }
        let gender = switch prefix.last {
        case "f": "♀"
        case "m": "♂"
        default: ""
        }
        return gender.isEmpty ? lang : "\(lang) \(gender)"
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
