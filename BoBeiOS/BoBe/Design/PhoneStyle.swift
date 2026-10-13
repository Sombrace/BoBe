import SwiftUI

enum PhoneText {
    case hero
    case title
    case heading
    case body
    case callout
    case footnote
    case label
    case digits

    var font: Font {
        switch self {
        case .hero: .system(.largeTitle, design: .rounded, weight: .semibold)
        case .title: .system(.title2, design: .rounded, weight: .semibold)
        case .heading: .system(.headline, design: .rounded)
        case .body: .body
        case .callout: .callout
        case .footnote: .footnote
        case .label: .system(.caption, design: .rounded, weight: .semibold)
        case .digits: .system(.title, design: .rounded, weight: .semibold).monospacedDigit()
        }
    }
}

extension View {
    func phoneText(_ style: PhoneText) -> some View {
        self.font(style.font)
    }

    func phoneThemed() -> some View {
        modifier(PhoneThemed())
    }

    func fittedSheet() -> some View {
        modifier(FittedSheet())
    }
}

private struct FittedSheet: ViewModifier {
    @State private var height: CGFloat = 300

    func body(content: Content) -> some View {
        content
            .fixedSize(horizontal: false, vertical: true)
            .onGeometryChange(for: CGFloat.self) { proxy in
                proxy.size.height
            } action: { height in
                self.height = height
            }
            .presentationDetents([.height(self.height)])
            .presentationDragIndicator(.visible)
    }
}

extension ThemeConfig {
    static func phone(_ scheme: ColorScheme) -> ThemeConfig {
        let id: ThemeId = scheme == .dark ? .bauhausDark : .bauhaus
        return allThemes.first { $0.themeId == id } ?? allThemes[0]
    }
}

private struct PhoneThemed: ViewModifier {
    @Environment(\.colorScheme) private var scheme

    func body(content: Content) -> some View {
        let theme = ThemeConfig.phone(self.scheme)
        content
            .environment(\.theme, theme)
            .tint(theme.colors.primary)
    }
}

struct PhoneButtonStyle: ButtonStyle {
    enum Kind {
        case primary
        case secondary
        case quiet
        case destructive
    }

    var kind: Kind = .primary
    @Environment(\.theme) private var theme
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(.body, design: .rounded, weight: .semibold))
            .multilineTextAlignment(.center)
            .foregroundStyle(self.foreground)
            .frame(maxWidth: self.kind == .quiet ? nil : .infinity, minHeight: self.kind == .quiet ? 44 : 54)
            .padding(.horizontal, 20)
            .background { self.background(pressed: configuration.isPressed) }
            .contentShape(Capsule(style: .continuous))
            .opacity(self.isEnabled ? 1 : 0.45)
            .scaleEffect(configuration.isPressed ? 0.97 : 1)
            .animation(BobeMotion.snappy, value: configuration.isPressed)
    }

    @ViewBuilder
    private func background(pressed: Bool) -> some View {
        switch self.kind {
        case .primary:
            Capsule(style: .continuous)
                .fill(self.theme.colors.primary.mix(with: .black, by: pressed ? 0.12 : 0))
                .shadow(color: self.theme.colors.primary.opacity(self.isEnabled ? 0.28 : 0), radius: 10, y: 5)
        case .secondary:
            Capsule(style: .continuous)
                .fill(pressed ? self.theme.well : self.theme.colors.surface)
                .overlay { Capsule(style: .continuous).strokeBorder(self.theme.hairline, lineWidth: 1) }
        case .quiet:
            Capsule(style: .continuous)
                .fill(pressed ? self.theme.well : .clear)
        case .destructive:
            Capsule(style: .continuous)
                .fill(self.theme.colors.destructive.mix(with: .black, by: pressed ? 0.12 : 0))
        }
    }

    private var foreground: Color {
        switch self.kind {
        case .primary, .destructive: self.theme.onAccent
        case .secondary: self.theme.colors.text
        case .quiet: self.theme.colors.primary
        }
    }
}

extension ButtonStyle where Self == PhoneButtonStyle {
    static var phonePrimary: PhoneButtonStyle {
        PhoneButtonStyle(kind: .primary)
    }

    static var phoneSecondary: PhoneButtonStyle {
        PhoneButtonStyle(kind: .secondary)
    }

    static var phoneQuiet: PhoneButtonStyle {
        PhoneButtonStyle(kind: .quiet)
    }

    static var phoneDestructive: PhoneButtonStyle {
        PhoneButtonStyle(kind: .destructive)
    }
}

struct PhoneBackdrop: View {
    var glowCenter = UnitPoint(x: 0.5, y: 0.22)
    @Environment(\.theme) private var theme

    var body: some View {
        ZStack {
            self.theme.colors.background
            RadialGradient(
                colors: [self.theme.glow[0].opacity(self.theme.isDark ? 0.14 : 0.24), .clear],
                center: self.glowCenter,
                startRadius: 8,
                endRadius: 380
            )
            RadialGradient(
                colors: [self.theme.glow[2].opacity(self.theme.isDark ? 0.08 : 0.1), .clear],
                center: UnitPoint(x: 0.9, y: 0.95),
                startRadius: 8,
                endRadius: 320
            )
        }
        .ignoresSafeArea()
    }
}

struct PhoneSectionLabel: View {
    let text: String
    @Environment(\.theme) private var theme

    var body: some View {
        Text(self.text.uppercased())
            .phoneText(.label)
            .tracking(0.6)
            .foregroundStyle(self.theme.colors.textMuted)
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityAddTraits(.isHeader)
    }
}

struct PhoneCard<Content: View>: View {
    var padding: CGFloat = 16
    @ViewBuilder let content: Content

    var body: some View {
        self.content
            .padding(self.padding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .bobeRaised(cornerRadius: BobeRadius.card)
    }
}
