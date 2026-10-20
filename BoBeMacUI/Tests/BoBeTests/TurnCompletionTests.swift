@testable import BoBe
import Testing

@Suite("Turn completion")
struct TurnCompletionTests {
    @Test(arguments: ["can you check the", "I was thinking that maybe", "open the file and", "so, um", "remind me to,"])
    func trailingOffSoundsUnfinished(text: String) {
        #expect(TurnCompletion.looksUnfinished(text))
    }

    @Test(arguments: ["what time is it", "Open the calendar.", "is that right?", "thanks", ""])
    func completeThoughtsAreNotHeld(text: String) {
        #expect(!TurnCompletion.looksUnfinished(text))
    }

    @Test
    func ellipsisStillSoundsUnfinished() {
        #expect(TurnCompletion.looksUnfinished("I think..."))
        #expect(TurnCompletion.looksUnfinished("well…"))
    }

    @Test
    func heldSpeechIsJoinedWithWhatFollows() {
        #expect(TurnCompletion.joined(nil, "hello") == "hello")
        #expect(TurnCompletion.joined("open the", "calendar") == "open the calendar")
        #expect(TurnCompletion.joined("open the", "") == "open the")
    }
}
