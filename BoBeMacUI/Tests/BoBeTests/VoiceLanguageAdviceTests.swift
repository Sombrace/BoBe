@testable import BoBe
import Testing

@Suite("VoiceLanguageAdvice")
struct VoiceLanguageAdviceTests {
    @Test
    func pocketOnlyAdvertisesInstalledEnglishSupport() {
        #expect(VoiceTtsBackend.best(for: "en-GB") == .serverPocket)
        #expect(VoiceTtsBackend.serverPocket.speaks("en-US"))
        #expect(!VoiceTtsBackend.serverPocket.speaks("zh-CN"))
        #expect(!VoiceTtsBackend.serverPocket.speaks("ja"))
        #expect(VoiceTtsBackend.best(for: "ja") == .clientSupertonic)
        #expect(VoiceTtsBackend.best(for: "ko") == .clientSupertonic)
    }

    @Test
    func recommendsAnAvailableEngineWithoutInventingLanguageSupport() {
        #expect(VoiceLanguageAdvice.advice(language: "ko", backend: .serverPocket, persona: "alba") == .engine(.clientSupertonic))
        #expect(VoiceLanguageAdvice.advice(language: "zh", backend: .clientSupertonic, persona: "alba") == nil)
        #expect(!VoiceTtsBackend.best(for: "zh").speaks("zh"))
    }

    @Test
    func oldPresetsMoveToTheEnglishDefault() {
        #expect(VoiceLanguageAdvice.advice(language: "en", backend: .serverPocket, persona: "af_bella") == .persona("alba"))
        #expect(PocketVoices.slot(for: "en", like: "marius") == "marius")
        #expect(PocketVoices.slot(for: "ja", like: "alba") == nil)
    }

    @Test
    func aWorkingChoiceIsLeftAlone() {
        #expect(VoiceLanguageAdvice.advice(language: "en", backend: .serverPocket, persona: "alba") == nil)
        #expect(VoiceLanguageAdvice.advice(language: "ko", backend: .clientSupertonic, persona: "alba") == nil)
    }
}
