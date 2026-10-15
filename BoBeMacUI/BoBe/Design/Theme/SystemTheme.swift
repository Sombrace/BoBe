import AppKit
import SwiftUI

@MainActor
func themeById(_ id: ThemeId) -> ThemeConfig {
    if id == .system {
        return systemTheme()
    }
    return allThemes.first { $0.themeId == id } ?? allThemes[0]
}

@MainActor
func systemTheme() -> ThemeConfig {
    let isDark = NSApp.effectiveAppearance.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua
    let base = themeById(isDark ? .bauhausDark : .bauhaus)
    return ThemeConfig(
        themeId: .system,
        nameKey: "theme.system.name",
        descriptionKey: "theme.system.description",
        isDark: isDark,
        colors: base.colors,
        avatar: base.avatar,
        glow: base.glow
    )
}
