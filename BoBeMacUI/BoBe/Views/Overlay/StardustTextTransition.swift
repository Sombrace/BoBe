import SwiftUI

enum AmbientBubbleDismissalStyle: String {
    case fade
    case stardust
}

struct AmbientBubbleTransition: ViewModifier {
    let style: AmbientBubbleDismissalStyle
    let progress: Double

    func body(content: Content) -> some View {
        let dissolving = self.style == .stardust
        let textProgress = dissolving ? self.progress : 0
        let shellProgress = dissolving
            ? max(0, (self.progress - 0.62) / 0.38)
            : self.progress
        content
            .textRenderer(StardustTextRenderer(progress: textProgress))
            .opacity(1 - shellProgress)
    }
}

private struct StardustTextRenderer: TextRenderer, Animatable {
    var progress: Double

    var animatableData: Double {
        get { self.progress }
        set { self.progress = newValue }
    }

    func draw(layout: Text.Layout, in context: inout GraphicsContext) {
        let slices = Array(layout.flattenedRunSlices)
        let lastIndex = max(1, slices.count - 1)

        for (index, slice) in slices.enumerated() {
            let position = Double(index) / Double(lastIndex)
            let grain = Double((index &* 37) % 17) / 17
            let threshold = position * 0.64 + grain * 0.08
            let localProgress = min(1, max(0, (self.progress - threshold) / 0.28))

            var copy = context
            copy.opacity = 1 - UnitCurve.easeIn.value(at: localProgress)
            copy.draw(slice, options: .disablesSubpixelQuantization)
        }
    }
}

private extension Text.Layout {
    var flattenedRuns: some RandomAccessCollection<Text.Layout.Run> {
        self.flatMap(\.self)
    }

    var flattenedRunSlices: some RandomAccessCollection<Text.Layout.RunSlice> {
        self.flattenedRuns.flatMap(\.self)
    }
}
