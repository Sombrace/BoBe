import SwiftUI

enum BobeControlSize {
    case mini
    case small
    case regular

    var fontSize: CGFloat {
        switch self {
        case .mini: 11
        case .small: 12
        case .regular: 13
        }
    }

    var horizontalPadding: CGFloat {
        switch self {
        case .mini: 9
        case .small: 12
        case .regular: 16
        }
    }

    var height: CGFloat {
        switch self {
        case .mini: 22
        case .small: 27
        case .regular: 34
        }
    }
}

enum BobeMetrics {
    static let paneHorizontalPadding: CGFloat = 16
    static let paneTopPadding: CGFloat = 16
    static let listRowMinHeight: CGFloat = 50
    static let listRowCornerRadius: CGFloat = 14
    static let pageMaxWidth: CGFloat = 760
}

extension View {
    func bobeTextStyle(_ style: BobeTextStyle) -> some View {
        font(style.font)
    }
}

struct BobeSpinner: View {
    var size: CGFloat = 14
    var lineWidth: CGFloat = 2
    var color: Color?

    init(size: CGFloat = 14, lineWidth: CGFloat = 2, color: Color? = nil) {
        self.size = size
        self.lineWidth = lineWidth
        self.color = color
    }

    @Environment(\.theme) private var theme
    @State private var spinning = false

    var body: some View {
        ZStack {
            Circle()
                .stroke(self.theme.colors.border.opacity(0.6), lineWidth: self.lineWidth)
            Circle()
                .trim(from: 0.12, to: 0.82)
                .stroke(
                    self.color ?? self.theme.colors.primary,
                    style: StrokeStyle(lineWidth: self.lineWidth, lineCap: .round)
                )
                .rotationEffect(.degrees(self.spinning ? 360 : 0))
                .animation(BobeMotion.isReduced ? nil : .linear(duration: 0.85).repeatForever(autoreverses: false), value: self.spinning)
        }
        .frame(width: self.size, height: self.size)
        .onAppear { self.spinning = true }
        .onDisappear { self.spinning = false }
    }
}

struct BobeLinearProgressBar: View {
    let progress: Double
    var height: CGFloat = 7
    var tint: Color?

    @Environment(\.theme) private var theme

    private var clampedProgress: Double {
        min(max(self.progress, 0), 1)
    }

    var body: some View {
        GeometryReader { geo in
            Capsule(style: .circular)
                .fill(self.fillStyle)
                .frame(width: max(self.height, geo.size.width * self.clampedProgress))
        }
        .frame(height: self.height)
        .bobeWell(in: .bobePill)
        .animation(BobeMotion.soft, value: self.clampedProgress)
    }

    private var fillStyle: AnyShapeStyle {
        if let tint = self.tint {
            AnyShapeStyle(tint)
        } else {
            AnyShapeStyle(
                LinearGradient(
                    colors: [self.theme.glow[0], self.theme.colors.primary],
                    startPoint: .leading,
                    endPoint: .trailing
                )
            )
        }
    }
}

struct BobeSelectableRow<Content: View>: View {
    let isSelected: Bool
    @ViewBuilder let content: Content

    @Environment(\.theme) private var theme
    @State private var isHovered = false

    init(
        isSelected: Bool,
        @ViewBuilder content: () -> Content
    ) {
        self.isSelected = isSelected
        self.content = content()
    }

    var body: some View {
        HStack(spacing: 10) {
            self.content
        }
        .foregroundStyle(self.theme.colors.text)
        .frame(maxWidth: .infinity, minHeight: BobeMetrics.listRowMinHeight, alignment: .leading)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background {
            let shape = RoundedRectangle(cornerRadius: BobeMetrics.listRowCornerRadius, style: .continuous)
            if self.isSelected {
                shape
                    .fill(self.theme.colors.surface)
                    .shadow(color: self.theme.shadowTint.opacity(self.theme.isDark ? 0.3 : 0.1), radius: 4, x: 1, y: 2)
                    .overlay { shape.strokeBorder(self.theme.hairline, style: BobeStroke.hairline) }
            } else if self.isHovered {
                shape.fill(self.theme.well.opacity(0.7))
            }
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(self.isSelected ? .isSelected : [])
        .animation(BobeMotion.fade, value: self.isHovered)
        .animation(BobeMotion.standard, value: self.isSelected)
        .onHover { self.isHovered = $0 }
    }
}
