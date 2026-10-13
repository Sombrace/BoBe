@testable import BoBe
import Testing

@Suite("VoiceLanguageAdvice")
struct VoiceLanguageAdviceTests {
    @Test
    func eachLanguageGetsAnEngineThatCanSpeakIt() {
        #expect(VoiceTtsBackend.best(for: "en") == .serverKokoro)
        #expect(VoiceTtsBackend.best(for: "zh-CN") == .serverKokoro)
        #expect(VoiceTtsBackend.best(for: "ja") == .serverKokoro)
        #expect(VoiceTtsBackend.best(for: "ko") == .clientSupertonic)
        #expect(VoiceTtsBackend.best(for: "de-DE") == .clientSupertonic)
        #expect(!VoiceTtsBackend.clientSupertonic.speaks("zh"))
        #expect(!VoiceTtsBackend.serverKokoro.speaks("ko"))
    }

    @Test
    func anEngineThatCannotSpeakTheLanguageIsReplaced() {
        #expect(VoiceLanguageAdvice.advice(language: "ko", backend: .serverKokoro, persona: "af_bella") == .engine(.clientSupertonic))
        #expect(VoiceLanguageAdvice.advice(language: "zh", backend: .clientSupertonic, persona: "af_bella") == .engine(.serverKokoro))
    }

    @Test
    func aVoiceMadeForAnotherLanguageIsSwappedKeepingItsGender() {
        #expect(VoiceLanguageAdvice.advice(language: "zh", backend: .serverKokoro, persona: "af_bella") == .persona("zf_xiaobei"))
        #expect(VoiceLanguageAdvice.advice(language: "ja", backend: .serverKokoro, persona: "am_adam") == .persona("jm_kumo"))
        #expect(VoiceLanguageAdvice.advice(language: "es", backend: .serverKokoro, persona: "bf_emma") == .persona("ef_dora"))
    }

    @Test
    func aWorkingChoiceIsLeftAlone() {
        #expect(VoiceLanguageAdvice.advice(language: "en", backend: .serverKokoro, persona: "bf_emma") == nil)
        #expect(VoiceLanguageAdvice.advice(language: "en", backend: .clientSupertonic, persona: "af_bella") == nil)
        #expect(VoiceLanguageAdvice.advice(language: "ko", backend: .clientSupertonic, persona: "af_bella") == nil)
        #expect(VoiceLanguageAdvice.advice(language: "zh", backend: .serverKokoro, persona: "zm_yunxi") == nil)
    }
}
