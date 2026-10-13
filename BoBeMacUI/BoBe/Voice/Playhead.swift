import Foundation

struct PlayheadSpan: Equatable, Sendable {
    let chunkId: UInt64
    let endFrame: Int64
}

enum Playhead {
    static func startFrame(scheduledThrough: Int64, played: Int64) -> Int64 {
        max(scheduledThrough, played)
    }

    static func chunk(
        replySpans: some Sequence<PlayheadSpan>,
        played: Int64,
        nextReplyChunk: UInt64
    ) -> UInt64 {
        replySpans.first { $0.endFrame > played }?.chunkId ?? nextReplyChunk
    }

    static func audibleMilliseconds(played: Int64, audioStart: Int64?, sampleRate: Double) -> UInt64? {
        guard let audioStart, sampleRate > 0 else { return nil }
        return UInt64(max(0, Double(played - audioStart) * 1_000 / sampleRate))
    }
}
