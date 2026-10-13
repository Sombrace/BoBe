@testable import BoBe
import Foundation
import Testing

@Suite("Localization", .serialized)
struct LocalizationTests {
    @Test
    @MainActor
    func composerHintExistsInEverySupportedLocale() {
        defer { L10n.setLocaleOverride(nil) }
        for locale in BobeStore.supportedLocales {
            L10n.setLocaleOverride(locale)
            let key = "overlay.input.placeholder"
            #expect(L10n.tr(key) != key, "Missing \(key) in \(locale)")
        }
    }
    @Test
    @MainActor
    func runtimeSetupAndRecoveryCopyExistsInEverySupportedLocale() throws {
        let keys = [
            "setup.runtime.address.label",
            "setup.runtime.code.label",
            "setup.runtime.phase.starting",
            "setup.runtime.phase.verifying",
            "setup.runtime.retry_connection",
            "setup.runtime.pair_again",
            "runtime.issue.unreachable",
            "runtime.issue.credential_rejected",
            "runtime.issue.secure_verification",
            "runtime.issue.authority_changed",
            "runtime.issue.protocol_incompatible",
            "runtime.issue.access_incomplete",
            "runtime.issue.local_material",
            "runtime.issue.managed_startup",
            "runtime.action.retry_connection",
            "runtime.action.repair",
            "settings.runtime.repair",
            "setup.wizard.progress.value_format",
            "setup.voice.try_again",
            "setup.voice.continue_without",
            "setup.physical.title",
            "setup.physical.disconnected",
        ]

        defer { L10n.setLocaleOverride(nil) }
        for locale in BobeStore.supportedLocales {
            let bundle = try #require(self.catalogBundle(for: locale))
            for key in keys {
                let value = NSLocalizedString(
                    key,
                    tableName: "UI",
                    bundle: bundle,
                    value: key,
                    comment: ""
                )
                #expect(value != key, "Missing direct translation for \(key) in \(locale)")
            }
        }
    }

    private func catalogBundle(for locale: String) -> Bundle? {
        for candidate in [locale, locale.lowercased(), String(locale.prefix(2))] {
            if let url = Bundle.appResources.url(forResource: candidate, withExtension: "lproj"),
               let bundle = Bundle(url: url) {
                return bundle
            }
        }
        return nil
    }
}
