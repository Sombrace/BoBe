import Foundation
import os

private let lock = OSAllocatedUnfairLock<Bundle?>(initialState: nil)

enum L10n {
    static func setLocaleOverride(_ localeId: String?) {
        let bundle = self.resolveBundle(for: localeId)
        lock.withLock { $0 = bundle }
    }

    static func tr(_ key: String, _ args: CVarArg...) -> String {
        let bundle = lock.withLock { $0 } ?? Bundle.appResources
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

    private static func resolveBundle(for localeId: String?) -> Bundle? {
        guard let localeId, !localeId.isEmpty else { return nil }
        let candidates = [localeId, localeId.lowercased(), String(localeId.prefix(2))]
        for candidate in candidates {
            if let url = Bundle.appResources.url(forResource: candidate, withExtension: "lproj"),
               let bundle = Bundle(url: url) {
                return bundle
            }
        }
        return nil
    }
}
