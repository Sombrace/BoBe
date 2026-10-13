import Foundation

enum BargeInVerdict: Equatable, Sendable {
    case interrupt
    case acknowledgement
    case unclear
}

enum BargeInPolicy {
    static let minimumWords = 2
    static let minimumUnspacedCharacters = 4
    static let maximumAcknowledgementWords = 4
    static let echoGuardMilliseconds: UInt64 = 350
    static let duckedVolume: Float = 0.3
    static let unduckAfterSeconds: TimeInterval = 0.7

    private static let acknowledgements: [[String]] = [
        ["mm", "hmm"], ["uh", "huh"], ["i", "see"], ["got", "it"], ["all", "right"], ["thank", "you"],
        ["makes", "sense"], ["of", "course"],
        ["yeah"], ["yes"], ["yep"], ["yup"], ["ok"], ["okay"], ["right"], ["sure"], ["mhm"], ["mhmm"], ["mmhmm"], ["uhhuh"], ["mm"], ["hmm"],
        ["hm"], ["oh"], ["ah"], ["cool"], ["nice"], ["great"], ["true"], ["exactly"], ["alright"], ["thanks"],
    ]

    private static let interruptionOpeners: Set<String> = [
        "wait", "no", "nope", "but", "actually", "hold", "hang", "sorry", "excuse", "hey", "listen", "bobe",
    ]

    static func judge(_ partial: String) -> BargeInVerdict {
        let words = self.spokenWords(partial)
        if self.isAcknowledgement(words) { return .acknowledgement }
        if let first = words.first, self.interruptionOpeners.contains(first) { return .interrupt }
        if words.count >= self.minimumWords { return .interrupt }
        let unspaced = partial.unicodeScalars.count(where: self.isUnspacedScript)
        return unspaced >= self.minimumUnspacedCharacters ? .interrupt : .unclear
    }

    private static func spokenWords(_ partial: String) -> [String] {
        partial.split(whereSeparator: { $0.isWhitespace || $0 == "-" || $0 == "\u{2013}" || $0 == "\u{2014}" })
            .map { String($0.lowercased().unicodeScalars.filter(CharacterSet.alphanumerics.contains)) }
            .filter { !$0.isEmpty }
    }

    private static func isAcknowledgement(_ words: [String]) -> Bool {
        guard !words.isEmpty, words.count <= self.maximumAcknowledgementWords else { return false }
        var rest = words[...]
        while !rest.isEmpty {
            guard let phrase = self.acknowledgements.first(where: { rest.starts(with: $0) }) else { return false }
            rest = rest.dropFirst(phrase.count)
        }
        return true
    }

    private static func isUnspacedScript(_ scalar: Unicode.Scalar) -> Bool {
        switch scalar.value {
        case 0x3040 ... 0x30FF, 0x3400 ... 0x4DBF, 0x4E00 ... 0x9FFF, 0xF900 ... 0xFAFF: true
        default: false
        }
    }
}
