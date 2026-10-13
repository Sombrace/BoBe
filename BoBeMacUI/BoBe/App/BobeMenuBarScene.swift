import AppKit
import Foundation
import Observation

@MainActor
final class BobeStatusItemController: NSObject, NSMenuDelegate {
    private let store: BobeStore
    private let menu = NSMenu()
    private(set) var statusItem: NSStatusItem?
    private var statusObservationGeneration: UInt64 = 0

    init(store: BobeStore = .shared) {
        self.store = store
        super.init()
    }

    func setup() {
        guard self.statusItem == nil else { return }

        let statusItem = NSStatusBar.system.statusItem(
            withLength: NSStatusItem.squareLength
        )
        self.menu.delegate = self
        self.menu.autoenablesItems = false
        statusItem.menu = self.menu
        self.statusItem = statusItem
        self.rebuildMenu()
        self.observeStatus()
    }

    func tearDown() {
        self.statusObservationGeneration &+= 1
        guard let statusItem else { return }
        NSStatusBar.system.removeStatusItem(statusItem)
        self.statusItem = nil
    }

    var menuItemTitles: [String] {
        self.menu.items.map(\.title)
    }

    nonisolated func menuNeedsUpdate(_ menu: NSMenu) {
        MainActor.assumeIsolated {
            self.rebuildMenu()
        }
    }

    private func observeStatus() {
        self.statusObservationGeneration &+= 1
        self.trackStatus(generation: self.statusObservationGeneration)
    }

    private func trackStatus(generation: UInt64) {
        guard generation == self.statusObservationGeneration else { return }
        withObservationTracking {
            _ = self.store.stateType
        } onChange: { [weak self] in
            Task { @MainActor [weak self] in
                guard let self,
                      generation == self.statusObservationGeneration
                else { return }
                self.configureButton(self.statusItem?.button)
                self.trackStatus(generation: generation)
            }
        }
    }

    private func rebuildMenu() {
        guard let statusItem else { return }
        self.configureButton(statusItem.button)
        self.menu.removeAllItems()

        if SetupWindowManager.shared.isOnboardingCompleted {
            self.menu.addItem(self.item(
                self.overlayToggleLabel, #selector(self.toggleOverlay), key: "b", modifiers: [.command, .shift],
                id: "status-menu.overlay"
            ))
            self.menu.addItem(self.item(
                L10n.tr("tray.settings"), #selector(self.openSettings), key: ",", id: "status-menu.settings"
            ))
        } else {
            self.menu.addItem(self.item(
                L10n.tr("app.setup_incomplete.retry"), #selector(self.openSetup), key: "b",
                modifiers: [.command, .shift], id: "status-menu.retry-setup"
            ))
        }

        let moreItem = NSMenuItem(title: L10n.tr("tray.more"), action: nil, keyEquivalent: "")
        moreItem.identifier = NSUserInterfaceItemIdentifier("status-menu.more")
        moreItem.submenu = self.buildMoreSubmenu()
        self.menu.addItem(moreItem)

        self.menu.addItem(.separator())
        self.menu.addItem(self.item(L10n.tr("tray.quit"), #selector(self.quitApp), key: "q", id: "status-menu.quit"))
    }

    private func item(
        _ title: String,
        _ action: Selector,
        key: String = "",
        modifiers: NSEvent.ModifierFlags = [.command],
        id: String
    ) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: key)
        item.target = self
        item.keyEquivalentModifierMask = modifiers
        item.identifier = NSUserInterfaceItemIdentifier(id)
        return item
    }

    private func buildMoreSubmenu() -> NSMenu {
        let submenu = NSMenu()
        submenu.autoenablesItems = false
        if SetupWindowManager.shared.isOnboardingCompleted {
            submenu.addItem(self.item(self.captureToggleLabel, #selector(self.toggleCapture), id: "status-menu.capture"))
            submenu.addItem(self.item(
                L10n.tr("tray.physical_setup"), #selector(self.openPhysicalBobeSetup), id: "status-menu.physical-bobe"
            ))
        }
        let languageItem = NSMenuItem(title: L10n.tr("tray.language"), action: nil, keyEquivalent: "")
        languageItem.identifier = NSUserInterfaceItemIdentifier("status-menu.language")
        languageItem.submenu = self.buildLanguageSubmenu()
        submenu.addItem(languageItem)
        submenu.addItem(.separator())
        let updates = self.item(L10n.tr("tray.check_updates"), #selector(self.checkForUpdates), id: "status-menu.updates")
        updates.isEnabled = BobeCommandRouter.shared.canCheckForUpdates
        submenu.addItem(updates)
        submenu.addItem(self.item(L10n.tr("tray.about"), #selector(self.showAbout), id: "status-menu.about"))
        return submenu
    }

    private func configureButton(_ button: NSStatusBarButton?) {
        guard let button else { return }
        let accessibilityLabel = L10n.tr("tray.status.accessibility")
        let status = self.statusLabel
        button.image = BobeStatusIcon.image(for: self.store.stateType)
        button.imagePosition = .imageOnly
        button.setAccessibilityIdentifier("bobe.status-item")
        button.setAccessibilityLabel(accessibilityLabel)
        button.setAccessibilityValue(status)
        button.toolTip = "\(accessibilityLabel) — \(status)"
    }

    private func buildLanguageSubmenu() -> NSMenu {
        let submenu = NSMenu()
        let systemDefault = NSMenuItem(
            title: L10n.tr("tray.language.system_default"),
            action: #selector(self.changeLanguage(_:)),
            keyEquivalent: ""
        )
        systemDefault.target = self
        systemDefault.representedObject = ""
        systemDefault.state = self.store.localeOverride.isEmpty ? .on : .off
        submenu.addItem(systemDefault)
        submenu.addItem(.separator())

        for localeId in BobeStore.supportedLocales {
            let item = NSMenuItem(
                title: Self.localeDisplayName(localeId),
                action: #selector(self.changeLanguage(_:)),
                keyEquivalent: ""
            )
            item.target = self
            item.representedObject = localeId
            item.state = self.store.localeOverride == localeId ? .on : .off
            submenu.addItem(item)
        }
        return submenu
    }

    @objc
    private func toggleOverlay() {
        BobeCommandRouter.shared.toggleOverlay()
        self.rebuildMenu()
    }

    @objc
    private func toggleCapture() {
        Task {
            await BobeCommandRouter.shared.toggleCapture()
            self.rebuildMenu()
        }
    }

    @objc
    private func openSettings() {
        BobeCommandRouter.shared.openSettings()
    }

    @objc
    private func openPhysicalBobeSetup() {
        SetupWindowManager.shared.show(initialStep: .physicalBoBe)
    }

    @objc
    private func openSetup() {
        SetupWindowManager.shared.show()
    }

    @objc
    private func changeLanguage(_ sender: NSMenuItem) {
        self.store.updateLocale(sender.representedObject as? String ?? "")
        self.rebuildMenu()
    }

    @objc
    private func checkForUpdates() {
        BobeCommandRouter.shared.checkForUpdates()
    }

    @objc
    private func showAbout() {
        BobeCommandRouter.shared.showAbout()
    }

    @objc
    private func quitApp() {
        BobeCommandRouter.shared.quit()
    }

    private var statusLabel: String {
        switch self.store.stateType {
        case .loading: L10n.tr("tray.state.connecting")
        case .idle: L10n.tr("tray.state.idle")
        case .thinking: L10n.tr("tray.state.thinking")
        case .speaking: L10n.tr("tray.state.speaking")
        case .wantsToSpeak: L10n.tr("tray.state.wants_to_speak")
        case .error: L10n.tr("tray.state.error")
        case .shuttingDown: L10n.tr("tray.state.shutting_down")
        }
    }

    private var overlayToggleLabel: String {
        BobeCommandRouter.shared.overlayTitle
    }

    private var captureToggleLabel: String {
        BobeCommandRouter.shared.captureTitle
    }

    private static func localeDisplayName(_ identifier: String) -> String {
        let locale = Locale(identifier: identifier)
        guard let native = locale.localizedString(forLanguageCode: identifier) else {
            return identifier
        }
        return native.prefix(1).uppercased() + native.dropFirst()
    }
}

enum BobeStatusIcon {
    private static let side: CGFloat = 18

    static func image(for state: BobeStateType) -> NSImage {
        let image = NSImage(size: NSSize(width: self.side, height: self.side), flipped: false) { rect in
            self.draw(state, in: rect)
            return true
        }
        image.isTemplate = true
        image.accessibilityDescription = L10n.tr("tray.status.accessibility")
        return image
    }

    private static func draw(_ state: BobeStateType, in rect: NSRect) {
        let center = NSPoint(x: rect.midX, y: rect.midY)
        let orbit: CGFloat = 7.4
        let ring = NSBezierPath(ovalIn: NSRect(
            x: center.x - orbit, y: center.y - orbit, width: orbit * 2, height: orbit * 2
        ))
        ring.lineWidth = 1.35
        switch state {
        case .loading, .error, .shuttingDown:
            ring.lineCapStyle = .round
            ring.setLineDash([1.6, 2.5], count: 2, phase: 0)
        default:
            break
        }
        NSColor.black.setStroke()
        NSColor.black.setFill()

        let moonRadius: CGFloat = state == .wantsToSpeak ? 2.7 : 2.0
        let angle: CGFloat = state == .wantsToSpeak ? .pi / 4 : -.pi / 4.7
        let moonCenter = NSPoint(x: center.x + orbit * cos(angle), y: center.y + orbit * sin(angle))
        let gap = NSBezierPath(ovalIn: NSRect(
            x: moonCenter.x - moonRadius - 1.1, y: moonCenter.y - moonRadius - 1.1,
            width: (moonRadius + 1.1) * 2, height: (moonRadius + 1.1) * 2
        ))
        NSGraphicsContext.saveGraphicsState()
        let clip = NSBezierPath(rect: rect)
        clip.append(gap)
        clip.windingRule = .evenOdd
        clip.addClip()
        ring.stroke()
        NSGraphicsContext.restoreGraphicsState()
        NSBezierPath(ovalIn: NSRect(
            x: moonCenter.x - moonRadius, y: moonCenter.y - moonRadius, width: moonRadius * 2, height: moonRadius * 2
        )).fill()

        let ball: CGFloat = 4.7
        let face = NSBezierPath(ovalIn: NSRect(x: center.x - ball, y: center.y - ball, width: ball * 2, height: ball * 2))
        let eye: CGFloat = state == .thinking ? 0.85 : 1.0
        let lift: CGFloat = state == .thinking ? 1.3 : 0.5
        for side in [-1.0, 1.0] {
            face.append(NSBezierPath(ovalIn: NSRect(
                x: center.x + CGFloat(side) * 1.75 - eye, y: center.y + lift - eye, width: eye * 2, height: eye * 2
            )))
        }
        face.windingRule = .evenOdd
        face.fill()
    }
}
