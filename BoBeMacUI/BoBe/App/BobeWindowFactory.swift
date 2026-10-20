import AppKit
import SwiftUI

enum BobeWindowFactory {
    static func make(
        contentRect: NSRect,
        styleMask: NSWindow.StyleMask,
        title: String,
        titlebarAppearsTransparent: Bool = true,
        titleVisibility: NSWindow.TitleVisibility = .hidden,
        animationBehavior: NSWindow.AnimationBehavior = .none,
        rootView: some View
    ) -> NSWindow {
        let theme = ThemeStore.shared.currentTheme

        let window = NSWindow(
            contentRect: contentRect,
            styleMask: styleMask,
            backing: .buffered,
            defer: false
        )
        window.title = title
        window.isRestorable = false
        window.titlebarAppearsTransparent = titlebarAppearsTransparent
        window.titleVisibility = titleVisibility
        window.isMovableByWindowBackground = true
        window.animationBehavior = animationBehavior
        window.backgroundColor = NSColor(theme.colors.background)
        window.contentViewController = NSHostingController(rootView: rootView)
        window.setContentSize(contentRect.size)
        window.center()
        return window
    }
}
