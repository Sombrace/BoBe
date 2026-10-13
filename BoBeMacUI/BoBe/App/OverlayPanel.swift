import AppKit

final class PassthroughContentView: NSView {
    override func hitTest(_ point: NSPoint) -> NSView? {
        let hit = super.hitTest(point)
        return hit === self ? nil : hit
    }
}

final class OverlayPanel: NSPanel {
    var onCancel: (() -> Void)?
    var allowsKey = false

    init(contentRect: NSRect, title: String, identifier: String) {
        super.init(
            contentRect: contentRect,
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )

        let passthrough = PassthroughContentView(frame: NSRect(origin: .zero, size: contentRect.size))
        passthrough.autoresizingMask = [.width, .height]
        passthrough.wantsLayer = true
        passthrough.layer?.masksToBounds = false
        contentView = passthrough

        self.title = title
        setAccessibilityIdentifier(identifier)
        setAccessibilityLabel(title)
        isOpaque = false
        backgroundColor = .clear
        hasShadow = false
        level = .floating
        isMovable = false
        isMovableByWindowBackground = false
        acceptsMouseMovedEvents = true
        hidesOnDeactivate = false
        isReleasedWhenClosed = false

        collectionBehavior = [
            .canJoinAllSpaces,
            .fullScreenAuxiliary,
        ]

        ignoresMouseEvents = true
    }

    override var canBecomeKey: Bool {
        self.allowsKey
    }

    override var canBecomeMain: Bool {
        false
    }

    override func cancelOperation(_ sender: Any?) {
        self.onCancel?()
    }
}
