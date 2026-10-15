@testable import BoBe
import Foundation
import Testing

@Suite("Localization")
struct LocalizationTests {
    @Test
    @MainActor
    func everySupportedLocaleResolvesToItsOwnCatalog() throws {
        for localeId in BobeStore.supportedLocales {
            let catalog = try #require(L10n.catalog(for: localeId), "No catalog for \(localeId)")
            #expect(catalog.localeId == localeId)
            #expect(catalog.bundle.bundleURL.pathExtension == "lproj")
        }
        #expect(L10n.catalog(for: "") == nil)
        #expect(L10n.catalog(for: "xx-XX") == nil)
    }

    @Test
    func voiceLanguagesUseTheSystemNameInTheInterfaceLanguage() {
        #expect(VoiceLanguages.displayName(for: "ja", in: Locale(identifier: "de-DE")) == "Japanisch")
        #expect(VoiceLanguages.displayName(for: "en", in: Locale(identifier: "el-GR")) == "Αγγλικά")
        #expect(VoiceLanguages.displayName(for: "ko", in: Locale(identifier: "ja-JP")) == "韓国語")
        #expect(VoiceLanguages.displayName(for: "zh", in: Locale(identifier: "en")) == "Chinese")
    }

    @Test
    func voiceLanguageMenuTitlesAreCapitalizedWhileSentencesKeepTheSystemCase() {
        let spanish = Locale(identifier: "es-ES")
        #expect(VoiceLanguages.displayName(for: "en", in: spanish) == "inglés")
        #expect(VoiceLanguages.menuTitle(for: "en", in: spanish) == "Inglés")
        #expect(VoiceLanguages.menuTitle(for: "el", in: Locale(identifier: "zh-CN")) == "希腊语")
    }

    @Test
    func voiceLanguagesWithoutASystemNameFallBackToTheirCode() {
        #expect(VoiceLanguages.displayName(for: "xx", in: Locale(identifier: "fr-FR")) == "xx")
        #expect(VoiceLanguages.menuTitle(for: "xx", in: Locale(identifier: "fr-FR")) == "xx")
    }
}
