import AppKit
import SwiftUI

final class SettingsWindowManager: NSObject, NSWindowDelegate {
    static let shared = SettingsWindowManager()

    private var window: NSWindow?

    override private init() {}

    func show(initialCategory: SettingsCategory? = nil) {
        if let window {
            window.title = L10n.tr("settings.window.title")
            window.makeKeyAndOrderFront(nil)
            NSApp.activate()
            if let initialCategory {
                SettingsEditCoordinator.shared.requestTransition {
                    let frame = window.frame
                    window.contentViewController = NSHostingController(
                        rootView: SettingsWindow(initialCategory: initialCategory)
                    )
                    window.setFrame(frame, display: true)
                }
            }
            return
        }

        let screen = NSScreen.main?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
        let initialSize = Self.initialSize(for: screen)

        let window = BobeWindowFactory.make(
            contentRect: NSRect(origin: .zero, size: initialSize),
            styleMask: [.titled, .closable, .resizable, .miniaturizable, .fullSizeContentView],
            title: L10n.tr("settings.window.title"),
            rootView: SettingsWindow(initialCategory: initialCategory)
        )
        window.backgroundColor = NSColor(ThemeStore.shared.currentTheme.ground)
        window.setAccessibilityIdentifier("window.settings")
        window.minSize = Self.minimumSize(for: screen)
        window.setFrameAutosaveName("BobeSettingsWindow")
        if window.setFrameUsingName("BobeSettingsWindow") {
            window.setFrame(
                Self.clampedFrame(
                    window.frame,
                    to: screen,
                    minimumSize: window.minSize
                ),
                display: false
            )
        }
        window.delegate = self
        window.makeKeyAndOrderFront(nil)
        NSApp.activate()
        self.window = window
    }

    static func initialSize(for screen: NSRect) -> NSSize {
        let availableWidth = max(760, screen.width - 40)
        let availableHeight = max(520, screen.height - 40)
        return NSSize(
            width: min(max(screen.width * 0.72, min(1200, availableWidth)), 1500, availableWidth),
            height: min(max(screen.height * 0.78, min(800, availableHeight)), 1100, availableHeight)
        )
    }

    static func minimumSize(for screen: NSRect) -> NSSize {
        NSSize(
            width: min(1100, max(760, screen.width - 40)),
            height: min(720, max(520, screen.height - 40))
        )
    }

    static func clampedFrame(
        _ frame: NSRect,
        to visibleFrame: NSRect,
        minimumSize: NSSize
    ) -> NSRect {
        let width = min(
            max(frame.width, minimumSize.width),
            visibleFrame.width
        )
        let height = min(
            max(frame.height, minimumSize.height),
            visibleFrame.height
        )
        let minX = visibleFrame.minX
        let maxX = visibleFrame.maxX - width
        let minY = visibleFrame.minY
        let maxY = visibleFrame.maxY - height
        return NSRect(
            x: min(max(frame.minX, minX), maxX),
            y: min(max(frame.minY, minY), maxY),
            width: width,
            height: height
        )
    }

    func close() {
        SettingsEditCoordinator.shared.requestTransition { [weak self] in
            self?.window?.delegate = nil
            self?.window?.close()
            self?.window = nil
        }
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        SettingsEditCoordinator.shared.requestTransition {
            sender.orderOut(nil)
        }
        return false
    }

    func windowWillClose(_ notification: Notification) {
        guard let closingWindow = notification.object as? NSWindow, closingWindow == self.window else { return }
        self.window = nil
    }
}
