import CoreGraphics
import Testing
@testable import BoBe

struct OverlayPlacementTests {
    private let screen = CGRect(x: 0, y: 0, width: 1512, height: 945)

    @Test
    func avatarWindowHugsTheAvatar() {
        let rect = OverlayCanvas.clusterRect(OverlayPlacement())
        #expect(rect.size == OverlayCanvas.clusterSize)
        #expect(rect.width == 150)
        #expect(rect.height == 164)
        #expect(OverlayCanvas.avatar.x - rect.minX == 72)
        #expect(OverlayCanvas.avatar.y - rect.minY == 86)
    }

    @Test
    func avatarStaysPutWhateverTheColumnDoes() {
        let avatar = CGPoint(x: 1434, y: 215)
        let cluster = OverlayCanvas.clusterRect(OverlayPlacement())
        let frame = OverlayCanvas.windowFrame(for: cluster, avatar: avatar)
        #expect(OverlayCanvas.avatarPoint(forWindowFrame: frame, viewport: cluster) == avatar)
        let column = OverlayCanvas.columnViewport(size: CGSize(width: 364, height: 520), placement: OverlayPlacement())
        let columnFrame = OverlayCanvas.windowFrame(for: column, avatar: avatar)
        #expect(OverlayCanvas.viewport(forWindowFrame: columnFrame, avatar: avatar) == column)
        #expect(columnFrame.maxX <= frame.minX + WindowSizes.shadowInset)
        #expect(columnFrame.width == 364 + WindowSizes.shadowInset * 2)
    }

    @Test(arguments: [
        OverlayPlacement(side: .leading, growth: .up),
        OverlayPlacement(side: .trailing, growth: .down),
    ])
    func columnWindowIsAViewportOnItsCanvas(placement: OverlayPlacement) {
        let canvas = OverlayCanvas.columnCanvasRect(placement)
        #expect(canvas.size == OverlayCanvas.columnCanvas)
        let viewport = OverlayCanvas.columnViewport(size: CGSize(width: 304, height: 120), placement: placement)
        #expect(canvas.contains(viewport))
        let origin = OverlayCanvas.columnHostOrigin(viewport: viewport, placement: placement)
        #expect(origin.x <= 0)
        #expect(origin.y <= 0)
        #expect(origin.x + canvas.width >= viewport.width)
        #expect(origin.y + canvas.height >= viewport.height)
    }

    @Test
    func columnOpensTowardTheRoomierSide() {
        let right = OverlayCanvas.place(avatar: CGPoint(x: 1434, y: 215), screen: self.screen)
        #expect(right.side == .leading)
        #expect(right.growth == .up)
        #expect(!right.satellitesBelow)

        let left = OverlayCanvas.place(avatar: CGPoint(x: 90, y: 215), screen: self.screen)
        #expect(left.side == .trailing)

        let high = OverlayCanvas.place(avatar: CGPoint(x: 1434, y: 900), screen: self.screen)
        #expect(high.growth == .down)
        #expect(high.satellitesBelow)

        let middle = OverlayCanvas.place(avatar: CGPoint(x: 1434, y: 623), screen: self.screen)
        #expect(middle.growth == .down)
        #expect(!middle.satellitesBelow)
    }

    @Test
    func mirroredColumnsSitOnTheirSideOfTheAvatar() {
        let size = CGSize(width: 364, height: 300)
        let leading = OverlayCanvas.columnBox(size: size, placement: OverlayPlacement(side: .leading, growth: .up))
        let trailing = OverlayCanvas.columnBox(size: size, placement: OverlayPlacement(side: .trailing, growth: .down))
        #expect(leading.maxX < OverlayCanvas.avatar.x)
        #expect(leading.maxY > OverlayCanvas.avatar.y)
        #expect(trailing.minX > OverlayCanvas.avatar.x)
        #expect(trailing.minY < OverlayCanvas.avatar.y)
        #expect(OverlayCanvas.avatar.y - trailing.minY == leading.maxY - OverlayCanvas.avatar.y)
    }

    @Test
    func heightLimitFollowsTheRoomAroundTheAvatar() {
        let low = OverlayCanvas.columnHeightLimit(avatar: CGPoint(x: 1434, y: 215), screen: self.screen, growth: .up)
        let high = OverlayCanvas.columnHeightLimit(avatar: CGPoint(x: 1434, y: 700), screen: self.screen, growth: .up)
        #expect(low > high)
        #expect(high > 0)
        #expect(low <= OverlayCanvas.columnStage.height)
    }

    @Test
    func draggedAvatarIsKeptOnScreen() {
        let clamped = OverlayCanvas.clamp(avatar: CGPoint(x: 1600, y: -40), screen: self.screen)
        #expect(self.screen.insetBy(dx: OverlayCanvas.avatarClearance - 1, dy: OverlayCanvas.avatarClearance - 1).contains(clamped))
        let inside = CGPoint(x: 700, y: 400)
        #expect(OverlayCanvas.clamp(avatar: inside, screen: self.screen) == inside)
    }

    @Test
    func bubbleTailPointsAtTheAvatar() {
        #expect(OverlayPlacement(side: .leading, growth: .up).corner == .bottomTrailing)
        #expect(OverlayPlacement(side: .trailing, growth: .down).corner == .topLeading)
    }

    @Test
    func toolGlyphsNameTheKindOfWork() {
        #expect(ToolGlyph.symbol(for: "web_search") == "magnifyingglass")
        #expect(ToolGlyph.symbol(for: "bash") == "terminal.fill")
        #expect(ToolGlyph.symbol(for: "read_file") == "doc.text.fill")
        #expect(ToolGlyph.symbol(for: "mystery") == "wrench.and.screwdriver.fill")
    }
}
