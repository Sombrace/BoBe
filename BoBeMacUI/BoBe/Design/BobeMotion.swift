import SwiftUI

enum BobeMotion {
    static var isReduced: Bool {
        AccessibilityRuntime.shared.reduceMotion
    }

    static var standard: Animation? {
        self.isReduced ? nil : .interpolatingSpring(mass: 0.9, stiffness: 420, damping: 34)
    }

    static var soft: Animation? {
        self.isReduced ? nil : .interpolatingSpring(mass: 1, stiffness: 260, damping: 30)
    }

    static var snappy: Animation? {
        self.isReduced ? nil : .interpolatingSpring(mass: 1, stiffness: 620, damping: 40)
    }

    static var face: Animation? {
        self.isReduced ? nil : .interpolatingSpring(mass: 1, stiffness: 250, damping: 22)
    }

    static var body: Animation? {
        self.isReduced ? nil : .interpolatingSpring(mass: 1, stiffness: 430, damping: 21)
    }

    static var fade: Animation? {
        self.isReduced ? nil : .timingCurve(0.16, 1, 0.3, 1, duration: 0.2)
    }

    static var gaze: Animation? {
        self.isReduced ? nil : .interpolatingSpring(mass: 1, stiffness: 180, damping: 20)
    }

    static var disclose: Animation? {
        self.isReduced ? nil : .spring(response: 0.32, dampingFraction: 0.9)
    }

    static var hover: Animation? {
        self.isReduced ? nil : .easeOut(duration: 0.15)
    }

    static var follow: Animation? {
        self.isReduced ? nil : .linear(duration: 0.12)
    }

    static var spin: Animation? {
        self.isReduced ? nil : .linear(duration: 0.85).repeatForever(autoreverses: false)
    }

    static var recede: Animation {
        self.isReduced ? .easeOut(duration: 0.16) : .easeIn(duration: 0.56)
    }

    static var restore: Animation {
        .easeOut(duration: self.isReduced ? 0.1 : 0.14)
    }

    static var windowFadeSeconds: TimeInterval {
        self.isReduced ? 0 : 0.18
    }

    static var symbolReplace: ContentTransition {
        self.isReduced ? .identity : .symbolEffect(.replace)
    }

    static func stagger(_ index: Int, step: Double = 0.04) -> Animation? {
        self.standard?.delay(Double(index) * step)
    }

    static func drift(_ index: Int) -> Animation? {
        self.isReduced ? nil : .easeOut(duration: 2.4).delay(Double(index) * 0.5)
    }

    static func hoverScale(_ isHovered: Bool) -> CGFloat {
        isHovered && !self.isReduced ? 1.02 : 1
    }
}

struct BobeRise: ViewModifier {
    let isActive: Bool
    var distance: CGFloat = 8
    var blur: CGFloat = 3

    func body(content: Content) -> some View {
        content
            .opacity(self.isActive ? 0 : 1)
            .offset(y: self.isActive ? self.distance : 0)
            .blur(radius: self.isActive ? self.blur : 0)
    }
}

extension AnyTransition {
    static var bobeRise: AnyTransition {
        guard !BobeMotion.isReduced else { return .opacity }
        return .asymmetric(
            insertion: .modifier(
                active: BobeRise(isActive: true),
                identity: BobeRise(isActive: false)
            ),
            removal: .modifier(
                active: BobeRise(isActive: true, distance: -6, blur: 2),
                identity: BobeRise(isActive: false)
            )
        )
    }

    static func bobePop(anchor: UnitPoint = .center, scale: CGFloat = 0.82) -> AnyTransition {
        guard !BobeMotion.isReduced else { return .opacity }
        return .scale(scale: scale, anchor: anchor).combined(with: .opacity)
    }
}
