import Foundation
import os

struct LocalizationCatalog: Sendable {
    let localeId: String
    let bundle: Bundle
}

private let lock = OSAllocatedUnfairLock<LocalizationCatalog?>(initialState: nil)

enum L10n {
    static func setLocaleOverride(_ localeId: String?) {
        let catalog = localeId.flatMap(self.catalog(for:))
        lock.withLock { $0 = catalog }
    }

    static var locale: Locale {
        let localeId = lock.withLock { $0?.localeId }
        return Locale(identifier: localeId ?? Bundle.appResources.preferredLocalizations.first ?? "en")
    }

    static func tr(_ key: String, _ args: CVarArg...) -> String {
        let bundle = lock.withLock { $0?.bundle } ?? Bundle.appResources
        var format = NSLocalizedString(
            key,
            tableName: "UI",
            bundle: bundle,
            value: key,
            comment: ""
        )
        if format == key,
           bundle.bundleURL != Bundle.appResources.bundleURL,
           let englishURL = Bundle.appResources.url(forResource: "en", withExtension: "lproj"),
           let englishBundle = Bundle(url: englishURL) {
            format = NSLocalizedString(
                key,
                tableName: "UI",
                bundle: englishBundle,
                value: key,
                comment: ""
            )
        }
        guard !args.isEmpty else { return format }
        return String(format: format, locale: .current, arguments: args)
    }

    static func catalog(for localeId: String) -> LocalizationCatalog? {
        guard !localeId.isEmpty else { return nil }
        let candidates = [localeId, localeId.lowercased(), String(localeId.prefix(2))]
        for candidate in candidates {
            if let url = Bundle.appResources.url(forResource: candidate, withExtension: "lproj"),
               let bundle = Bundle(url: url) {
                return LocalizationCatalog(localeId: localeId, bundle: bundle)
            }
        }
        return nil
    }
}
