import SwiftUI

enum ThemeId: String, CaseIterable, Sendable {
    case system
    case bauhaus
    case bauhausPastel = "bauhaus-pastel"
    case cute
    case cutePastel = "cute-pastel"
    case bauhausDark = "bauhaus-dark"
    case cuteDark = "cute-dark"
}

struct ThemeColors: Sendable {
    let primary: Color
    let secondary: Color
    let tertiary: Color
    let background: Color
    let surface: Color
    let border: Color
    let text: Color
    let textMuted: Color
    let success: Color
    let warning: Color
    let error: Color
    let destructive: Color
}

struct ThemeConfig: Identifiable, Sendable {
    var id: ThemeId {
        self.themeId
    }

    let themeId: ThemeId
    let nameKey: String
    let descriptionKey: String
    let isDark: Bool
    let colors: ThemeColors
    let avatar: BobeAvatarPalette
    let glow: [Color]

    var name: String {
        L10n.tr(self.nameKey)
    }

    var description: String {
        L10n.tr(self.descriptionKey)
    }
}

let allThemes: [ThemeConfig] = [
    ThemeConfig(
        themeId: .bauhaus,
        nameKey: "theme.bauhaus.name",
        descriptionKey: "theme.bauhaus.description",
        isDark: false,
        colors: ThemeColors(
            primary: Color(hex: "94472E"),
            secondary: Color(hex: "526A48"),
            tertiary: Color(hex: "70584C"),
            background: Color(hex: "FAF7F2"),
            surface: Color(hex: "FFFDFC"),
            border: Color(hex: "D8C9B4"),
            text: Color(hex: "3A3A3A"),
            textMuted: Color(hex: "62605D"),
            success: Color(hex: "2F6F48"),
            warning: Color(hex: "805A00"),
            error: Color(hex: "B3261E"),
            destructive: Color(hex: "B3261E")
        ),
        avatar: .terracotta,
        glow: [Color(rgb: 0xF4C56A), Color(rgb: 0xE88A45), Color(rgb: 0xE0706F)]
    ),
    ThemeConfig(
        themeId: .bauhausPastel,
        nameKey: "theme.bauhaus_pastel.name",
        descriptionKey: "theme.bauhaus_pastel.description",
        isDark: false,
        colors: ThemeColors(
            primary: Color(hex: "865046"),
            secondary: Color(hex: "53694D"),
            tertiary: Color(hex: "6B5C52"),
            background: Color(hex: "FDFBF8"),
            surface: Color(hex: "FFFFFF"),
            border: Color(hex: "DED3C4"),
            text: Color(hex: "4A4A4A"),
            textMuted: Color(hex: "66625F"),
            success: Color(hex: "2F6F48"),
            warning: Color(hex: "805A00"),
            error: Color(hex: "B3261E"),
            destructive: Color(hex: "B3261E")
        ),
        avatar: .softClay,
        glow: [Color(rgb: 0xE9C58A), Color(rgb: 0xC98A63), Color(rgb: 0xC97A78)]
    ),
    ThemeConfig(
        themeId: .cute,
        nameKey: "theme.cute.name",
        descriptionKey: "theme.cute.description",
        isDark: false,
        colors: ThemeColors(
            primary: Color(hex: "A33E5A"),
            secondary: Color(hex: "3D715F"),
            tertiary: Color(hex: "66518A"),
            background: Color(hex: "FFF8FA"),
            surface: Color(hex: "FFFFFF"),
            border: Color(hex: "E9CFD7"),
            text: Color(hex: "3D3D3D"),
            textMuted: Color(hex: "646062"),
            success: Color(hex: "2F6F48"),
            warning: Color(hex: "805A00"),
            error: Color(hex: "B3261E"),
            destructive: Color(hex: "B3261E")
        ),
        avatar: .bubblegum,
        glow: [Color(rgb: 0xFFC27A), Color(rgb: 0xF2789B), Color(rgb: 0xC77BD8)]
    ),
    ThemeConfig(
        themeId: .cutePastel,
        nameKey: "theme.cute_pastel.name",
        descriptionKey: "theme.cute_pastel.description",
        isDark: false,
        colors: ThemeColors(
            primary: Color(hex: "963D56"),
            secondary: Color(hex: "426B5B"),
            tertiary: Color(hex: "62527B"),
            background: Color(hex: "FFFCFD"),
            surface: Color(hex: "FFFFFF"),
            border: Color(hex: "EBD8DE"),
            text: Color(hex: "4D4D4D"),
            textMuted: Color(hex: "686466"),
            success: Color(hex: "2F6F48"),
            warning: Color(hex: "805A00"),
            error: Color(hex: "B3261E"),
            destructive: Color(hex: "B3261E")
        ),
        avatar: .cottonCandy,
        glow: [Color(rgb: 0xF7B8D8), Color(rgb: 0xB08AE0), Color(rgb: 0x7FA8F0)]
    ),
    ThemeConfig(
        themeId: .bauhausDark,
        nameKey: "theme.bauhaus_dark.name",
        descriptionKey: "theme.bauhaus_dark.description",
        isDark: true,
        colors: ThemeColors(
            primary: Color(hex: "D4926F"),
            secondary: Color(hex: "9AAD8E"),
            tertiary: Color(hex: "B8A090"),
            background: Color(hex: "1E1E1E"),
            surface: Color(hex: "2A2A2A"),
            border: Color(hex: "3D3D3D"),
            text: Color(hex: "E8E4DF"),
            textMuted: Color(hex: "B3AEA9"),
            success: Color(hex: "82C995"),
            warning: Color(hex: "E4B95A"),
            error: Color(hex: "FF8A80"),
            destructive: Color(hex: "FF8A80")
        ),
        avatar: .midnightClay,
        glow: [Color(rgb: 0xF4C56A), Color(rgb: 0xE88A45), Color(rgb: 0xE0706F)]
    ),
    ThemeConfig(
        themeId: .cuteDark,
        nameKey: "theme.cute_dark.name",
        descriptionKey: "theme.cute_dark.description",
        isDark: true,
        colors: ThemeColors(
            primary: Color(hex: "F2A0B0"),
            secondary: Color(hex: "8DCAB5"),
            tertiary: Color(hex: "C4B4E0"),
            background: Color(hex: "1A1A1E"),
            surface: Color(hex: "252528"),
            border: Color(hex: "3A3A40"),
            text: Color(hex: "F5F0F2"),
            textMuted: Color(hex: "B2ADB0"),
            success: Color(hex: "82C995"),
            warning: Color(hex: "E4B95A"),
            error: Color(hex: "FF8A80"),
            destructive: Color(hex: "FF8A80")
        ),
        avatar: .twilightRose,
        glow: [Color(rgb: 0xFFB3C4), Color(rgb: 0xF2789B), Color(rgb: 0xB79AF0)]
    ),
]

extension ThemeConfig {
    var well: Color {
        self.isDark
            ? self.colors.background.mix(with: .black, by: 0.32)
            : self.colors.background.mix(with: self.colors.text, by: 0.055)
    }

    var ground: Color {
        self.isDark
            ? self.colors.background.mix(with: .black, by: 0.22)
            : self.colors.background.mix(with: self.colors.text, by: 0.035)
    }

    var accentSoft: Color {
        self.colors.primary.mix(with: self.colors.surface, by: self.isDark ? 0.76 : 0.86)
    }

    var hairline: Color {
        self.colors.border.opacity(self.isDark ? 0.8 : 0.65)
    }

    var shadowTint: Color {
        self.isDark ? .black : Color(rgb: 0x2D1E0F)
    }

    var highlight: Color {
        self.isDark ? Color(rgb: 0xFFF1DC).opacity(0.09) : .white.opacity(0.9)
    }

    var onAccent: Color {
        self.isDark ? self.colors.background : Color(rgb: 0xFFFDFC)
    }

    var glowGradient: [Color] {
        self.glow + [self.glow[0]]
    }
}

extension EnvironmentValues {
    @Entry var theme: ThemeConfig = allThemes[0]
}

extension Color {
    init(hex: String) {
        let hex = hex.trimmingCharacters(in: CharacterSet.alphanumerics.inverted)
        var int: UInt64 = 0
        Scanner(string: hex).scanHexInt64(&int)
        let r: Double
        let g: Double
        let b: Double
        switch hex.count {
        case 6:
            r = Double((int >> 16) & 0xFF) / 255.0
            g = Double((int >> 8) & 0xFF) / 255.0
            b = Double(int & 0xFF) / 255.0
        default:
            r = 1
            g = 1
            b = 1
        }
        self.init(red: r, green: g, blue: b)
    }
}
