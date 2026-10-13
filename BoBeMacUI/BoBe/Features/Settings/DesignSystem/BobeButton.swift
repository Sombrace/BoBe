import SwiftUI

enum BobeButtonVariant {
    case primary
    case secondary
    case ghost
    case destructive
}

struct BobeButtonStyle: ButtonStyle {
    let variant: BobeButtonVariant
    let size: BobeControlSize
    let hovered: Bool

    @Environment(\.theme) private var theme
    @Environment(\.isEnabled) private var isEnabled

    init(
        variant: BobeButtonVariant = .secondary,
        size: BobeControlSize = .regular,
        hovered: Bool = false
    ) {
        self.variant = variant
        self.size = size
        self.hovered = hovered
    }

    func makeBody(configuration: Configuration) -> some View {
        let isPressed = configuration.isPressed

        configuration.label
            .font(.system(size: self.size.fontSize, weight: .semibold))
            .foregroundStyle(self.foregroundColor)
            .lineLimit(1)
            .padding(.horizontal, self.size.horizontalPadding)
            .frame(minHeight: self.size.height)
            .background {
                Capsule(style: .circular)
                    .fill(self.backgroundColor(isPressed: isPressed))
                    .shadow(
                        color: self.theme.shadowTint.opacity(self.castsShadow ? 0.14 : 0),
                        radius: self.hovered ? 4 : 1.5,
                        y: self.hovered ? 2 : 1
                    )
            }
            .overlay {
                Capsule(style: .circular)
                    .strokeBorder(self.borderColor, style: BobeStroke.hairline)
            }
            .opacity(self.isEnabled ? 1 : 0.45)
            .scaleEffect(isPressed ? 0.96 : 1)
            .animation(BobeMotion.snappy, value: isPressed)
            .animation(BobeMotion.fade, value: self.hovered)
            .contentShape(Capsule(style: .circular))
    }

    private var castsShadow: Bool {
        self.isEnabled && (self.variant == .primary || self.variant == .secondary || self.variant == .destructive)
    }

    private var foregroundColor: Color {
        switch self.variant {
        case .primary, .destructive:
            self.theme.onAccent
        case .secondary:
            self.theme.colors.text
        case .ghost:
            self.hovered ? self.theme.colors.text : self.theme.colors.textMuted
        }
    }

    private func backgroundColor(isPressed: Bool) -> Color {
        switch self.variant {
        case .primary:
            self.theme.colors.primary.mix(with: .white, by: self.hovered && !isPressed ? 0.08 : 0)
        case .secondary:
            isPressed ? self.theme.well : self.theme.colors.surface
        case .ghost:
            isPressed || self.hovered ? self.theme.well : .clear
        case .destructive:
            self.theme.colors.destructive.mix(with: .white, by: self.hovered && !isPressed ? 0.08 : 0)
        }
    }

    private var borderColor: Color {
        switch self.variant {
        case .secondary: self.theme.hairline
        default: .clear
        }
    }
}

private struct BobeButtonModifier: ViewModifier {
    let variant: BobeButtonVariant
    let size: BobeControlSize
    @State private var isHovered = false

    func body(content: Content) -> some View {
        content
            .buttonStyle(BobeButtonStyle(variant: self.variant, size: self.size, hovered: self.isHovered))
            .onHover { self.isHovered = $0 }
    }
}

extension View {
    func bobeButton(
        _ variant: BobeButtonVariant = .secondary,
        size: BobeControlSize = .regular
    ) -> some View {
        modifier(BobeButtonModifier(variant: variant, size: size))
    }
}
