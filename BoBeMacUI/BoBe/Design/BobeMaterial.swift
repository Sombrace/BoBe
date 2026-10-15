import SwiftUI

enum BobeElevation {
    case resting
    case lifted
}

enum BobeStroke {
    static let hairline = StrokeStyle(lineWidth: 1, lineCap: .round, lineJoin: .round)
}

enum BobeRadius {
    static let chip: CGFloat = 10
    static let control: CGFloat = 14
    static let field: CGFloat = 18
    static let card: CGFloat = 22
    static let capsule: CGFloat = 23
    static let sheet: CGFloat = 28
}

extension Shape where Self == Capsule {
    static var bobePill: Capsule {
        Capsule(style: .circular)
    }
}

private struct BobeRaisedSurface<S: InsettableShape>: ViewModifier {
    let shape: S
    let elevation: BobeElevation
    let fill: Color?
    @Environment(\.theme) private var theme

    func body(content: Content) -> some View {
        content
            .background {
                self.shape
                    .fill(self.fill ?? self.theme.colors.surface)
                    .shadow(color: self.tint(self.near.opacity), radius: self.near.radius, x: self.near.x, y: self.near.y)
                    .shadow(color: self.tint(self.far.opacity), radius: self.far.radius, x: self.far.x, y: self.far.y)
            }
            .overlay {
                self.shape.strokeBorder(self.theme.hairline, style: BobeStroke.hairline)
            }
            .overlay {
                self.shape
                    .inset(by: 1)
                    .strokeBorder(
                        LinearGradient(
                            colors: [self.theme.highlight, self.theme.highlight.opacity(0)],
                            startPoint: .topLeading,
                            endPoint: UnitPoint(x: 0.62, y: 0.62)
                        ),
                        style: BobeStroke.hairline
                    )
                    .allowsHitTesting(false)
            }
    }

    private var darkBoost: Double {
        self.theme.isDark ? 2.2 : 1
    }

    private func tint(_ opacity: Double) -> Color {
        self.theme.shadowTint.opacity(opacity)
    }

    private var near: BobeShadow {
        switch self.elevation {
        case .resting: BobeShadow(opacity: 0.09 * self.darkBoost, radius: 3, x: 1, y: 2)
        case .lifted: BobeShadow(opacity: 0.12 * self.darkBoost, radius: 6, x: 2, y: 4)
        }
    }

    private var far: BobeShadow {
        switch self.elevation {
        case .resting: BobeShadow(opacity: 0.07 * self.darkBoost, radius: 10, x: 3, y: 7)
        case .lifted: BobeShadow(opacity: 0.11 * self.darkBoost, radius: 20, x: 6, y: 14)
        }
    }
}

private struct BobeShadow {
    let opacity: Double
    let radius: CGFloat
    let x: CGFloat
    let y: CGFloat
}

private struct BobeWellSurface<S: InsettableShape>: ViewModifier {
    let shape: S
    @Environment(\.theme) private var theme

    func body(content: Content) -> some View {
        content
            .background {
                self.shape.fill(
                    self.theme.well.shadow(
                        .inner(color: self.theme.shadowTint.opacity(self.theme.isDark ? 0.4 : 0.1), radius: 1.5, y: 1)
                    )
                )
            }
            .overlay {
                self.shape.strokeBorder(
                    self.theme.shadowTint.opacity(self.theme.isDark ? 0.3 : 0.04),
                    style: BobeStroke.hairline
                )
            }
    }
}

extension View {
    func bobeRaised(
        in shape: some InsettableShape,
        elevation: BobeElevation = .resting,
        fill: Color? = nil
    ) -> some View {
        modifier(BobeRaisedSurface(shape: shape, elevation: elevation, fill: fill))
    }

    func bobeRaised(
        cornerRadius: CGFloat = BobeRadius.card,
        elevation: BobeElevation = .resting,
        fill: Color? = nil
    ) -> some View {
        self.bobeRaised(
            in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous),
            elevation: elevation,
            fill: fill
        )
    }

    func bobeWell(in shape: some InsettableShape) -> some View {
        modifier(BobeWellSurface(shape: shape))
    }

    func bobeWell(cornerRadius: CGFloat = BobeRadius.field) -> some View {
        self.bobeWell(in: RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
    }
}

struct BobePressStyle: ButtonStyle {
    var scale: CGFloat = 0.96

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? self.scale : 1)
            .animation(BobeMotion.snappy, value: configuration.isPressed)
    }
}

struct BobeIconTile: View {
    let systemName: String
    let tint: Color
    var size: CGFloat = 30

    var body: some View {
        RoundedRectangle(cornerRadius: self.size * 0.3, style: .continuous)
            .fill(
                LinearGradient(
                    colors: [self.tint.mix(with: .white, by: 0.18), self.tint],
                    startPoint: .top,
                    endPoint: .bottom
                )
            )
            .overlay {
                RoundedRectangle(cornerRadius: self.size * 0.3, style: .continuous)
                    .strokeBorder(
                        LinearGradient(
                            colors: [.white.opacity(0.35), .black.opacity(0.08)],
                            startPoint: .top,
                            endPoint: .bottom
                        ),
                        lineWidth: 1
                    )
            }
            .overlay {
                Image(systemName: self.systemName)
                    .font(.system(size: self.size * 0.48, weight: .semibold))
                    .foregroundStyle(.white)
            }
            .frame(width: self.size, height: self.size)
            .shadow(color: self.tint.opacity(0.28), radius: 3, y: 2)
            .accessibilityHidden(true)
    }
}

struct BobeShimmerText: View {
    let text: String
    var font: Font = .system(size: 13, weight: .medium)
    @Environment(\.theme) private var theme
    @State private var sweep = false

    var body: some View {
        Text(self.text)
            .font(self.font)
            .foregroundStyle(self.theme.colors.textMuted)
            .overlay {
                if !BobeMotion.isReduced {
                    GeometryReader { proxy in
                        LinearGradient(
                            colors: [.clear, self.theme.glow[1], .clear],
                            startPoint: .leading,
                            endPoint: .trailing
                        )
                        .frame(width: max(40, proxy.size.width * 0.5))
                        .offset(x: self.sweep ? proxy.size.width * 1.1 : -proxy.size.width * 0.6)
                    }
                    .mask {
                        Text(self.text).font(self.font)
                    }
                    .allowsHitTesting(false)
                }
            }
            .lineLimit(1)
            .task(id: self.text) {
                self.sweep = false
                guard !BobeMotion.isReduced else { return }
                withAnimation(.timingCurve(0.2, 0.7, 0.2, 1, duration: 2.2).repeatForever(autoreverses: false)) {
                    self.sweep = true
                }
            }
    }
}

struct BobeWorkingRim<S: InsettableShape>: View {
    let shape: S
    var lineWidth: CGFloat = 1.5
    @Environment(\.theme) private var theme
    @State private var turn = false

    var body: some View {
        GeometryReader { proxy in
            let side = hypot(proxy.size.width, proxy.size.height)
            ZStack {
                self.wheel(side: side, in: proxy.size)
                    .mask { self.shape.strokeBorder(lineWidth: self.lineWidth + 4) }
                    .blur(radius: 7)
                    .opacity(0.5)
                self.wheel(side: side, in: proxy.size)
                    .mask { self.shape.strokeBorder(lineWidth: self.lineWidth) }
            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        .task {
            guard !BobeMotion.isReduced else { return }
            withAnimation(.linear(duration: 3.2).repeatForever(autoreverses: false)) {
                self.turn = true
            }
        }
    }

    private func wheel(side: CGFloat, in size: CGSize) -> some View {
        AngularGradient(colors: self.theme.glowGradient, center: .center)
            .frame(width: side, height: side)
            .rotationEffect(.degrees(self.turn ? 360 : 0))
            .frame(width: size.width, height: size.height)
    }
}

struct BobeBreathingDot: View {
    var size: CGFloat = 7
    @Environment(\.theme) private var theme
    @State private var inhale = false

    var body: some View {
        Circle()
            .fill(self.theme.glow[1])
            .frame(width: self.size, height: self.size)
            .scaleEffect(self.inhale ? 1.25 : 1)
            .opacity(self.inhale ? 0.5 : 0.95)
            .task {
                guard !BobeMotion.isReduced else { return }
                withAnimation(.easeInOut(duration: 0.9).repeatForever(autoreverses: true)) {
                    self.inhale = true
                }
            }
            .accessibilityHidden(true)
    }
}
