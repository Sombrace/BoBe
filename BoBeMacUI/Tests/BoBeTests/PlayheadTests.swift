@testable import BoBe
import Testing

struct PlayheadTests {
    private let spans = [
        PlayheadSpan(chunkId: 0, endFrame: 24_000),
        PlayheadSpan(chunkId: 1, endFrame: 60_000),
        PlayheadSpan(chunkId: 2, endFrame: 72_000),
    ]

    @Test
    func audioScheduledAfterASilentWaitStartsAtThePlayhead() {
        #expect(Playhead.startFrame(scheduledThrough: 0, played: 36_000) == 36_000)
        #expect(Playhead.startFrame(scheduledThrough: 60_000, played: 36_000) == 60_000)
    }

    @Test
    func playheadNamesTheFirstChunkThatHasNotFinished() {
        #expect(Playhead.chunk(replySpans: self.spans, played: 0, nextReplyChunk: 3) == 0)
        #expect(Playhead.chunk(replySpans: self.spans, played: 23_999, nextReplyChunk: 3) == 0)
        #expect(Playhead.chunk(replySpans: self.spans, played: 24_000, nextReplyChunk: 3) == 1)
        #expect(Playhead.chunk(replySpans: self.spans, played: 71_000, nextReplyChunk: 3) == 2)
    }

    @Test
    func playheadPastEverythingScheduledNamesTheNextChunk() {
        #expect(Playhead.chunk(replySpans: self.spans, played: 72_000, nextReplyChunk: 3) == 3)
        #expect(Playhead.chunk(replySpans: [PlayheadSpan](), played: 5_000, nextReplyChunk: 0) == 0)
    }

    @Test
    func echoGuardCountsFromTheFirstAudibleSample() {
        #expect(Playhead.audibleMilliseconds(played: 48_000, audioStart: nil, sampleRate: 24_000) == nil)
        #expect(Playhead.audibleMilliseconds(played: 48_000, audioStart: 43_200, sampleRate: 24_000) == 200)
        #expect(Playhead.audibleMilliseconds(played: 10, audioStart: 43_200, sampleRate: 24_000) == 0)
    }
}
