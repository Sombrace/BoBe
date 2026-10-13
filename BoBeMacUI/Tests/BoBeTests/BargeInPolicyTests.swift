@testable import BoBe
import Testing

@Suite("Barge-in policy")
struct BargeInPolicyTests {
    @Test
    func shortAgreementDoesNotTakeTheFloor() {
        for partial in ["yeah", "Mm-hmm.", "uh huh", "okay", "oh, I see", "got it", "Right, right."] {
            #expect(BargeInPolicy.judge(partial) == .acknowledgement, "\(partial)")
        }
    }

    @Test
    func realSpeechTakesTheFloor() {
        for partial in ["please stop now", "what about", "tell me more", "okay but why"] {
            #expect(BargeInPolicy.judge(partial) == .interrupt, "\(partial)")
        }
    }

    @Test
    func disagreementAndRedirectionNeedOnlyOneWord() {
        for partial in ["wait", "No.", "actually", "hold on", "Sorry"] {
            #expect(BargeInPolicy.judge(partial) == .interrupt, "\(partial)")
        }
    }

    @Test
    func noiseAndSingleWordsStayUnclear() {
        for partial in ["", "   ", "the", "weather"] {
            #expect(BargeInPolicy.judge(partial) == .unclear, "\(partial)")
        }
    }

    @Test
    func unspacedScriptsCountCharacters() {
        #expect(BargeInPolicy.judge("ちょっと待って") == .interrupt)
        #expect(BargeInPolicy.judge("等一下好吗") == .interrupt)
        #expect(BargeInPolicy.judge("はい") == .unclear)
    }
}
