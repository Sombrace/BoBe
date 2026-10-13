import SwiftUI

struct OverlayPlacement: Equatable, Sendable {
    enum Side: Sendable {
        case leading
        case trailing
    }

    enum Growth: Sendable {
        case up
        case down
    }

    var side: Side = .leading
    var growth: Growth = .up
    var satellitesBelow = false

    var alignment: Alignment {
        switch (self.side, self.growth) {
        case (.leading, .up): .bottomTrailing
        case (.leading, .down): .topTrailing
        case (.trailing, .up): .bottomLeading
        case (.trailing, .down): .topLeading
        }
    }

    var corner: UnitPoint {
        switch (self.side, self.growth) {
        case (.leading, .up): .bottomTrailing
        case (.leading, .down): .topTrailing
        case (.trailing, .up): .bottomLeading
        case (.trailing, .down): .topLeading
        }
    }

    var nearEdge: UnitPoint {
        self.side == .leading ? .trailing : .leading
    }

    var hugsTrailing: Bool {
        self.side == .leading
    }

    var nearEdgeAlignment: Alignment {
        self.side == .leading ? .trailing : .leading
    }

    var nearVerticalEdge: Edge.Set {
        self.growth == .up ? .bottom : .top
    }

    func bubbleShape(radius: CGFloat = 20, tail: CGFloat = 7) -> UnevenRoundedRectangle {
        let corner = self.corner
        return UnevenRoundedRectangle(
            topLeadingRadius: corner == .topLeading ? tail : radius,
            bottomLeadingRadius: corner == .bottomLeading ? tail : radius,
            bottomTrailingRadius: corner == .bottomTrailing ? tail : radius,
            topTrailingRadius: corner == .topTrailing ? tail : radius,
            style: .continuous
        )
    }
}

extension EnvironmentValues {
    @Entry var overlayPlacement = OverlayPlacement()
}

struct OverlayHitRegionKey: PreferenceKey {
    static let defaultValue: [CGRect] = []

    static func reduce(value: inout [CGRect], nextValue: () -> [CGRect]) {
        value.append(contentsOf: nextValue())
    }
}

extension View {
    func overlayHitRegion() -> some View {
        self.background {
            GeometryReader { proxy in
                Color.clear.preference(
                    key: OverlayHitRegionKey.self,
                    value: [proxy.frame(in: .named(OverlayCanvas.space))]
                )
            }
        }
    }
}

enum OverlayCanvas {
    static let clusterLeft: CGFloat = AvatarMetrics.center.x
    static let clusterRight: CGFloat = AvatarMetrics.clusterWidth - AvatarMetrics.center.x + WindowSizes.clusterTrailing
    static let clusterTall: CGFloat = AvatarMetrics.center.y
    static let clusterShort: CGFloat = AvatarMetrics.clusterHeight - AvatarMetrics.center.y + WindowSizes.clusterBottom
    static let columnDrop: CGFloat = AvatarMetrics.clusterHeight - AvatarMetrics.center.y - WindowSizes.columnBaseline
    static let avatarClearance: CGFloat = AvatarMetrics.cardSize / 2 + 8
    static let satelliteReach: CGFloat = AvatarMetrics.satelliteOrbit + AvatarMetrics.satelliteSize / 2 + 6
    static let space = "overlay.canvas"
    static let minimumColumnHeight: CGFloat = 180

    static let reach = CGSize(
        width: clusterLeft + WindowSizes.columnConversation + WindowSizes.shadowInset,
        height: WindowSizes.heightMax
    )
    static let avatar = CGPoint(x: reach.width, y: reach.height)
    static let columnStage = CGSize(
        width: WindowSizes.columnConversation,
        height: WindowSizes.heightMax - WindowSizes.shadowInset
    )

    static func clusterRect(_ placement: OverlayPlacement) -> CGRect {
        let above = placement.satellitesBelow ? self.clusterShort : self.clusterTall
        let below = placement.satellitesBelow ? self.clusterTall : self.clusterShort
        return CGRect(
            x: self.avatar.x - self.clusterLeft,
            y: self.avatar.y - above,
            width: self.clusterLeft + self.clusterRight,
            height: above + below
        )
    }

    static func columnBox(size: CGSize, placement: OverlayPlacement) -> CGRect {
        let x = placement.side == .leading
            ? self.avatar.x - self.clusterLeft - size.width
            : self.avatar.x + self.clusterLeft
        let y = placement.growth == .up
            ? self.avatar.y + self.columnDrop - size.height
            : self.avatar.y - self.columnDrop
        return CGRect(x: x, y: y, width: size.width, height: size.height)
    }

    static let clusterSize = CGSize(width: clusterLeft + clusterRight, height: clusterTall + clusterShort)
    static let columnCanvas = CGSize(
        width: columnStage.width + WindowSizes.shadowInset * 2,
        height: columnStage.height + WindowSizes.shadowInset * 2
    )

    static func columnCanvasRect(_ placement: OverlayPlacement) -> CGRect {
        self.columnBox(size: self.columnStage, placement: placement)
            .insetBy(dx: -WindowSizes.shadowInset, dy: -WindowSizes.shadowInset)
    }

    static func columnViewport(size: CGSize, placement: OverlayPlacement) -> CGRect {
        self.columnBox(size: size, placement: placement)
            .insetBy(dx: -WindowSizes.shadowInset, dy: -WindowSizes.shadowInset)
    }

    static func columnHostOrigin(viewport: CGRect, placement: OverlayPlacement) -> CGPoint {
        let canvas = self.columnCanvasRect(placement)
        return CGPoint(x: canvas.minX - viewport.minX, y: viewport.maxY - canvas.maxY)
    }

    static func windowFrame(for viewport: CGRect, avatar: CGPoint) -> CGRect {
        CGRect(
            x: avatar.x + viewport.minX - self.avatar.x,
            y: avatar.y - (viewport.maxY - self.avatar.y),
            width: viewport.width,
            height: viewport.height
        )
    }

    static func viewport(forWindowFrame frame: CGRect, avatar: CGPoint) -> CGRect {
        CGRect(
            x: frame.minX - avatar.x + self.avatar.x,
            y: self.avatar.y - (frame.maxY - avatar.y),
            width: frame.width,
            height: frame.height
        )
    }

    static func avatarPoint(forWindowFrame frame: CGRect, viewport: CGRect) -> CGPoint {
        CGPoint(
            x: frame.minX - viewport.minX + self.avatar.x,
            y: frame.minY + viewport.maxY - self.avatar.y
        )
    }

    static func columnHeightLimit(avatar: CGPoint, screen: CGRect, growth: OverlayPlacement.Growth) -> CGFloat {
        let room = growth == .up ? screen.maxY - avatar.y : avatar.y - screen.minY
        return min(
            self.columnStage.height,
            max(0, room + self.columnDrop - WindowSizes.shadowInset - WindowSizes.margin)
        )
    }

    static func place(avatar: CGPoint, screen: CGRect) -> OverlayPlacement {
        let leftRoom = avatar.x - screen.minX
        let rightRoom = screen.maxX - avatar.x
        let side: OverlayPlacement.Side = leftRoom >= self.reach.width || leftRoom >= rightRoom ? .leading : .trailing
        let up = self.columnHeightLimit(avatar: avatar, screen: screen, growth: .up)
        let down = self.columnHeightLimit(avatar: avatar, screen: screen, growth: .down)
        let comfortable = OverlayPresentation.ask.columnHeight
        let growth: OverlayPlacement.Growth = up >= comfortable || up >= down ? .up : .down
        let satellitesBelow = screen.maxY - avatar.y < self.clusterTall
        return OverlayPlacement(side: side, growth: growth, satellitesBelow: satellitesBelow)
    }

    static func clamp(avatar: CGPoint, screen: CGRect) -> CGPoint {
        let inset = self.avatarClearance
        guard screen.width > inset * 2, screen.height > inset * 2 else { return avatar }
        return CGPoint(
            x: min(max(avatar.x, screen.minX + inset), screen.maxX - inset).rounded(),
            y: min(max(avatar.y, screen.minY + inset), screen.maxY - inset).rounded()
        )
    }
}

struct AnchoredStack: Layout {
    var hugsTrailing = true
    var reversed = false
    var spacing: CGFloat = 8

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache _: inout ()) -> CGSize {
        let sizes = self.sizes(of: subviews, width: proposal.width)
        let occupied = sizes.filter { $0.height > 0 }
        let height = occupied.reduce(0) { $0 + $1.height } + CGFloat(max(0, occupied.count - 1)) * self.spacing
        let width = proposal.width ?? (sizes.map(\.width).max() ?? 0)
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache _: inout ()) {
        let sizes = self.sizes(of: subviews, width: bounds.width)
        let order = self.reversed ? Array(subviews.indices.reversed()) : Array(subviews.indices)
        var y = bounds.minY
        for index in order {
            let size = sizes[index]
            let x = self.hugsTrailing ? bounds.maxX - size.width : bounds.minX
            subviews[index].place(
                at: CGPoint(x: x, y: y),
                anchor: .topLeading,
                proposal: ProposedViewSize(width: bounds.width, height: nil)
            )
            if size.height > 0 {
                y += size.height + self.spacing
            }
        }
    }

    private func sizes(of subviews: Subviews, width: CGFloat?) -> [CGSize] {
        subviews.map { $0.sizeThatFits(ProposedViewSize(width: width, height: nil)) }
    }
}
