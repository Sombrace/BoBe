@testable import BoBe
import Testing

@Suite("VoiceLanguageAdvice")
struct VoiceLanguageAdviceTests {
    @Test(arguments: ["en", "en-GB", "en-US"])
    func pocketSpeaksEnglish(_ language: String) {
        #expect(VoiceTtsBackend.best(for: language) == .serverPocket)
    }

    @Test(arguments: ["el", "zh-CN", "ja", "ko", "es"])
    func otherLanguagesUseTheSystemVoice(_ language: String) {
        #expect(VoiceTtsBackend.best(for: language) == .systemVoice)
        #expect(VoiceLanguageAdvice.pocketVoice(language: language, persona: "alba") == nil)
    }

    @Test
    func oldPresetsMoveToTheEnglishDefault() {
        #expect(VoiceLanguageAdvice.pocketVoice(language: "en", persona: "af_bella") == "alba")
        #expect(PocketVoices.slot(for: "en", like: "marius") == "marius")
        #expect(PocketVoices.slot(for: "ja", like: "alba") == nil)
    }

    @Test
    func aWorkingChoiceIsLeftAlone() {
        #expect(VoiceLanguageAdvice.pocketVoice(language: "en", persona: "alba") == nil)
    }
}
