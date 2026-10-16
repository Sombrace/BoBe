import AppKit
@testable import BoBe
import Foundation
import SwiftUI
import Testing

@Suite("Overlay presentation")
struct OverlayPresentationTests {
    @Test
    func aReplyTallerThanTwiceTheViewportCountsAsPresentedOnceItFillsTheView() {
        let viewport = WindowSizes.heightChatViewportMax
        let reply = viewport * 2 + 80

        #expect(MessagePresentation.isPresented(visibleHeight: viewport, rowHeight: reply, viewportHeight: viewport))
        #expect(MessagePresentation.isPresented(visibleHeight: viewport / 2, rowHeight: reply, viewportHeight: viewport))
        #expect(!MessagePresentation.isPresented(visibleHeight: 24, rowHeight: reply, viewportHeight: viewport))
        #expect(MessagePresentation.isPresented(visibleHeight: 60, rowHeight: 100, viewportHeight: viewport))
        #expect(!MessagePresentation.isPresented(visibleHeight: 40, rowHeight: 100, viewportHeight: viewport))
        #expect(!MessagePresentation.isPresented(visibleHeight: 0, rowHeight: 0, viewportHeight: viewport))
    }

    @Test
    @MainActor
    func reachoutsTrimmedFromAFullConversationStopWaiting() {
        let selection = EndpointSelection(generation: 1, profileID: UUID(), authorityID: UUID(), endpoint: .unavailable)
        let store = BobeStore(currentSelection: { selection }, sendRequest: { _, _, _ in
            SendMessageResponse(messageId: "unused", replayed: false)
        })
        store.updateState { context in
            context.messages = (0 ..< BobeStore.messageWindowCapacity).map {
                ChatMessage(id: "message-\($0)", sender: .bobe, content: "\($0)", responseOrigin: .proactive)
            }
        }
        let model = OverlayModel()
        model.reachoutInbox.hold("message-0")
        model.reachoutKinds = ["message-0": .thought, "message-1": .report]
        model.floatingBubbleMessageId = "message-0"
        let before = store.messages.map(\.id)

        store.updateState { context in
            context.messages.append(ChatMessage(id: "message-new", sender: .bobe, content: "new"))
        }
        let after = store.messages.map(\.id)

        #expect(after.count == before.count)
        #expect(after != before)
        #expect(!after.contains("message-0"))

        model.forgetMessages(keeping: Set(after))

        #expect(model.reachoutInbox.isEmpty)
        #expect(model.reachoutKinds == ["message-1": .report])
        #expect(model.floatingBubbleMessageId == nil)
    }

    @Test
    func completedRepliesAreSpokenWithoutMarkdownSyntax() {
        #expect(
            CompletedReplyText.plainText(
                fromMarkdown: "**Bold** and `code` with [a link](https://example.com).\n\nNext step."
            ) == "Bold and code with a link.\nNext step."
        )
        #expect(CompletedReplyText.plainText(fromMarkdown: "# Plan\n\n- one\n- two *now*") == "Plan\none\ntwo now")
        #expect(CompletedReplyText.plainText(fromMarkdown: "Plain answer") == "Plain answer")
    }

    @Test
    @MainActor
    func noticeActionsMoveUnderALongMessage() {
        let width = WindowSizes.columnAmbient
        let message = "BoBe kann den Bildschirm nicht lesen, bis du die Bildschirmaufnahme erlaubst."
        let actions = [
            OverlayNoticeAction(id: "allow", title: "Bildschirmaufnahme erlauben", isProminent: true) {},
            OverlayNoticeAction(id: "off", title: "Bildschirmlesen ausschalten") {},
        ]
        let stacked = Self.fittingSize(
            OverlayNotice(systemName: "eye.slash", tone: .info, message: message, actions: actions, onDismiss: {}),
            width: width
        )
        let messageOnly = Self.fittingSize(
            OverlayNotice(systemName: "eye.slash", tone: .info, message: message, onDismiss: {}),
            width: width
        )
        let inline = Self.fittingSize(
            OverlayNotice(
                systemName: "exclamationmark",
                tone: .caution,
                message: "Not sent",
                actions: [OverlayNoticeAction(id: "retry", title: "Retry", isProminent: true) {}],
                onDismiss: {}
            ),
            width: width
        )
        let inlineMessageOnly = Self.fittingSize(
            OverlayNotice(systemName: "exclamationmark", tone: .caution, message: "Not sent", onDismiss: {}),
            width: width
        )

        #expect(stacked.width <= width + 0.5)
        #expect(stacked.height >= messageOnly.height + 26)
        #expect(abs(inline.height - inlineMessageOnly.height) < 1)
    }

    @MainActor
    private static func fittingSize(_ notice: OverlayNotice, width: CGFloat) -> CGSize {
        let host = NSHostingView(
            rootView: notice
                .frame(width: width)
                .environment(\.theme, allThemes[0])
        )
        return host.fittingSize
    }

    @Test
    @MainActor
    func replyCodeKeepsTheBoBeThemeColorsWhateverTheMacAppearance() throws {
        let light = try #require(allThemes.first { !$0.isDark })
        let dark = try #require(allThemes.first { $0.isDark })
        let systemCodeBackground = [
            ColorScheme.dark: SIMD3(37, 38, 42),
            ColorScheme.light: SIMD3(247, 247, 249),
        ]

        for (theme, macAppearance) in [(light, ColorScheme.dark), (dark, ColorScheme.light)] {
            let pixels = try Self.renderedPixels(
                CompletedReplyText(markdown: "Edit `settings.json` now.\n\n```\nlet value = 42\n```")
                    .padding(12)
                    .background(theme.colors.background)
                    .environment(\.theme, theme)
                    .environment(\.colorScheme, macAppearance),
                size: CGSize(width: 320, height: 180)
            )
            let well = try Self.srgb(theme.well)
            let wrong = try #require(systemCodeBackground[macAppearance])

            #expect(pixels.filter { Self.isNear($0, well) }.count > 40)
            #expect(!pixels.contains { Self.isNear($0, wrong) })
        }
    }

    @MainActor
    private static func renderedPixels(_ view: some View, size: CGSize) throws -> [SIMD3<Int>] {
        let host = NSHostingView(rootView: view.frame(width: size.width, height: size.height))
        let window = NSWindow(
            contentRect: CGRect(origin: .zero, size: size),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        window.contentView = host
        defer { window.close() }
        host.layoutSubtreeIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.2))
        let bitmap = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: bitmap)
        var pixels: [SIMD3<Int>] = []
        for y in stride(from: 0, to: bitmap.pixelsHigh, by: 2) {
            for x in stride(from: 0, to: bitmap.pixelsWide, by: 2) {
                if let color = bitmap.colorAt(x: x, y: y) {
                    pixels.append(try Self.srgb(color))
                }
            }
        }
        return pixels
    }

    private static func srgb(_ color: Color) throws -> SIMD3<Int> {
        try self.srgb(NSColor(color))
    }

    private static func srgb(_ color: NSColor) throws -> SIMD3<Int> {
        let converted = try #require(color.usingColorSpace(.sRGB))
        return SIMD3(
            Int((converted.redComponent * 255).rounded()),
            Int((converted.greenComponent * 255).rounded()),
            Int((converted.blueComponent * 255).rounded())
        )
    }

    private static func isNear(_ lhs: SIMD3<Int>, _ rhs: SIMD3<Int>) -> Bool {
        let difference = lhs &- rhs
        return abs(difference.x) <= 4 && abs(difference.y) <= 4 && abs(difference.z) <= 4
    }
}
