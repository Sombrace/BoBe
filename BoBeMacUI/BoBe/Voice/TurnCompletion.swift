import Foundation

enum TurnCompletion {
    static let graceMilliseconds = 1_100
    static let maximumHolds = 2

    private static let trailingOff: Set<String> = [
        "um", "uh", "er", "erm", "hmm", "like", "so", "and", "but", "or", "because", "cause", "if", "when",
        "while", "that", "which", "who", "the", "a", "an", "to", "of", "with", "for", "from", "in", "on", "at",
        "by", "about", "my", "your", "our", "their", "is", "are", "was", "were", "i", "we", "its", "im",
        "then", "also", "plus", "maybe", "well",
    ]

    static func looksUnfinished(_ text: String) -> Bool {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let last = trimmed.last else { return false }
        if ".?!。？！".contains(last) {
            return trimmed.hasSuffix("...") || trimmed.hasSuffix("…")
        }
        if ",;:-–—、，…".contains(last) { return true }
        let words = trimmed.split(whereSeparator: \.isWhitespace)
        guard let tail = words.last else { return false }
        let word = String(tail.lowercased().unicodeScalars.filter(CharacterSet.alphanumerics.contains))
        return self.trailingOff.contains(word)
    }

    static func joined(_ held: String?, _ next: String) -> String {
        guard let held, !held.isEmpty else { return next }
        return next.isEmpty ? held : "\(held) \(next)"
    }
}
