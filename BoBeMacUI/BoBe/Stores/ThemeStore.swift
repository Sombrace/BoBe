import AppKit
import Observation
import SwiftUI

@Observable
final class ThemeStore: NSObject {
    static let shared = ThemeStore()

    private(set) var themeId: ThemeId
    private var appearanceRevision = 0
    @ObservationIgnored private var appearanceObservation: NSKeyValueObservation?
    private static let themeDefaultsKey = "bobe_theme_id"

    var currentTheme: ThemeConfig {
        _ = self.appearanceRevision
        return themeById(self.themeId)
    }

    var preferredColorScheme: ColorScheme? {
        guard self.themeId != .system else { return nil }
        return self.currentTheme.isDark ? .dark : .light
    }

    override private init() {
        self.themeId =
            UserDefaults.standard.string(forKey: Self.themeDefaultsKey)
                .flatMap { ThemeId(rawValue: $0) } ?? .bauhaus
        super.init()
        self.appearanceObservation = NSApp.observe(
            \.effectiveAppearance,
            options: [.new]
        ) { [weak self] _, _ in
            Task { @MainActor [weak self] in
                self?.appearanceRevision &+= 1
            }
        }
    }

    func setTheme(_ id: ThemeId) {
        self.themeId = id
        UserDefaults.standard.set(id.rawValue, forKey: Self.themeDefaultsKey)
    }
}
