import AppKit
import SwiftUI

private final class OverlayHostingView<Content: View>: NSHostingView<Content> {
    override var mouseDownCanMoveWindow: Bool {
        false
    }

    override var safeAreaInsets: NSEdgeInsets {
        NSEdgeInsets()
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool {
        true
    }
}

@MainActor
@Observable
final class OverlayStage {
    var placement = OverlayPlacement()
    var columnHeightLimit: CGFloat = OverlayCanvas.columnStage.height
    var escapeCount = 0
    var clickAwayCount = 0
    var focusRequest = 0
    var pointerOverAvatar = false
    var pointerOverContent = false
}

@MainActor
final class OverlayWindowManager: NSObject {
    static let shared = OverlayWindowManager()

    let stage = OverlayStage()

    private(set) var panel: OverlayPanel?
    private(set) var anchor: CGPoint = .zero
    private(set) var requested: CGRect?

    private var columnPanel: OverlayPanel?
    private var columnHost: NSView?
    private var dragOrigin: CGPoint?
    private var chatOpen = false
    private var clickMonitors: [Any] = []
    private var pointerMonitors: [Any] = []
    private var hitRegions: [CGRect] = []
    private var avatarReach = OverlayCanvas.avatarClearance
    private var releaseTask: Task<Void, Never>?
    private var screenObserver: (any NSObjectProtocol)?

    private static let anchorXKey = "bobe.overlay.anchor_x"
    private static let anchorYKey = "bobe.overlay.anchor_y"
    private static let hitMargin: CGFloat = 14
    private static let releaseMilliseconds = 90

    override private init() {}

    func createPanels(avatar: some View, column: some View) {
        if self.panel != nil {
            self.close()
        }

        let saved = self.savedAnchor
        let screen = saved.flatMap { point in
            NSScreen.screens.first { $0.frame.insetBy(dx: -40, dy: -40).contains(point) }
        } ?? NSScreen.main ?? NSScreen.screens[0]
        let screenFrame = screen.visibleFrame
        let fallback = CGPoint(
            x: screenFrame.maxX - WindowSizes.margin - OverlayCanvas.clusterRight,
            y: screenFrame.minY + WindowSizes.margin + OverlayCanvas.clusterShort
        )
        self.anchor = OverlayCanvas.clamp(avatar: saved ?? fallback, screen: screenFrame)
        let placement = OverlayCanvas.place(avatar: self.anchor, screen: screenFrame)
        self.stage.placement = placement
        self.stage.columnHeightLimit = OverlayCanvas.columnHeightLimit(
            avatar: self.anchor, screen: screenFrame, growth: placement.growth
        )
        self.requested = nil
        self.chatOpen = false

        let avatarFrame = OverlayCanvas.windowFrame(for: OverlayCanvas.clusterRect(placement), avatar: self.anchor)
        let panel = OverlayPanel(contentRect: avatarFrame, title: "BoBe Overlay", identifier: "window.overlay")
        Self.install(
            OverlayHostingView(rootView: avatar.ignoresSafeArea()),
            size: OverlayCanvas.clusterSize,
            in: panel
        )

        let columnPanel = OverlayPanel(
            contentRect: NSRect(origin: avatarFrame.origin, size: NSSize(width: 1, height: 1)),
            title: "BoBe Conversation",
            identifier: "window.overlay.conversation"
        )
        columnPanel.onCancel = { [weak self] in self?.stage.escapeCount &+= 1 }
        self.columnHost = Self.install(
            OverlayHostingView(rootView: column.ignoresSafeArea()),
            size: OverlayCanvas.columnCanvas,
            in: columnPanel
        )
        self.columnPanel = columnPanel

        panel.orderFrontRegardless()
        self.panel = panel
        self.trackPointer(true)
        self.evaluatePointer()

        self.screenObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.settleAfterMove() }
        }
    }

    @discardableResult
    private static func install(
        _ host: OverlayHostingView<some View>,
        size: CGSize,
        in panel: NSPanel
    ) -> NSView {
        host.sizingOptions = []
        host.autoresizingMask = []
        host.frame = NSRect(origin: .zero, size: size)
        host.wantsLayer = true
        host.layer?.masksToBounds = false
        panel.contentView?.addSubview(host)
        return host
    }

    func reveal(_ rect: CGRect) {
        self.requested = self.requested?.union(rect) ?? rect
        self.applyColumn()
        self.evaluatePointer()
    }

    func settle(_ rect: CGRect?) {
        self.requested = rect
        self.applyColumn()
        self.evaluatePointer()
    }

    private func applyAvatar() {
        guard let panel else { return }
        let frame = OverlayCanvas.windowFrame(
            for: OverlayCanvas.clusterRect(self.stage.placement), avatar: self.anchor
        )
        if !Self.matches(panel.frame, frame) {
            panel.setFrame(frame, display: true)
        }
    }

    private func applyColumn() {
        guard let columnPanel, let columnHost else { return }
        guard let rect = self.requested, self.panel?.isVisible == true else {
            if columnPanel.isVisible {
                columnPanel.orderOut(nil)
            }
            return
        }
        let wanted = OverlayCanvas.windowFrame(for: rect, avatar: self.anchor)
        let visible = wanted.intersection(self.screenFrame)
        let frame = visible.isNull || visible.isEmpty ? wanted : visible
        let applied = OverlayCanvas.viewport(forWindowFrame: frame, avatar: self.anchor)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        columnHost.setFrameOrigin(OverlayCanvas.columnHostOrigin(viewport: applied, placement: self.stage.placement))
        if !Self.matches(columnPanel.frame, frame) {
            columnPanel.setFrame(frame, display: true)
        }
        CATransaction.commit()
        if !columnPanel.isVisible {
            self.orderColumnFront()
        }
    }

    private func orderColumnFront() {
        guard let columnPanel else { return }
        columnPanel.orderFrontRegardless()
        self.panel?.order(.above, relativeTo: columnPanel.windowNumber)
    }

    private static func matches(_ lhs: CGRect, _ rhs: CGRect) -> Bool {
        abs(lhs.minX - rhs.minX) < 0.5 && abs(lhs.minY - rhs.minY) < 0.5
            && abs(lhs.width - rhs.width) < 0.5 && abs(lhs.height - rhs.height) < 0.5
    }

    var screenFrame: CGRect {
        let screen = NSScreen.screens.first { $0.frame.contains(self.anchor) } ?? self.panel?.screen ?? NSScreen.main
        return screen?.visibleFrame ?? CGRect(x: 0, y: 0, width: 1_440, height: 900)
    }

    func show() {
        self.panel?.orderFrontRegardless()
        self.applyColumn()
        self.evaluatePointer()
    }

    func hide() {
        self.columnPanel?.orderOut(nil)
        self.panel?.orderOut(nil)
    }

    var isVisible: Bool {
        self.panel?.isVisible ?? false
    }

    func close() {
        self.watchClicksAway(false)
        self.trackPointer(false)
        self.releaseTask?.cancel()
        self.releaseTask = nil
        self.hitRegions = []
        self.dragOrigin = nil
        if let screenObserver {
            NotificationCenter.default.removeObserver(screenObserver)
            self.screenObserver = nil
        }
        self.columnPanel?.close()
        self.columnPanel = nil
        self.columnHost = nil
        self.panel?.close()
        self.panel = nil
    }

    func setHitRegions(_ regions: [CGRect]) {
        self.hitRegions = regions.filter { $0.width > 2 && $0.height > 2 }
        self.evaluatePointer()
    }

    func setSatellitesExposed(_ exposed: Bool) {
        self.avatarReach = exposed ? OverlayCanvas.satelliteReach : OverlayCanvas.avatarClearance
        self.evaluatePointer()
    }

    func setChatOpen(_ open: Bool) {
        guard open != self.chatOpen else { return }
        self.chatOpen = open
        self.watchClicksAway(open)
        self.columnPanel?.allowsKey = open
        if !open {
            self.releaseKeyboard()
        }
        self.evaluatePointer()
    }

    func focusForTyping() {
        guard let columnPanel else { return }
        columnPanel.allowsKey = true
        columnPanel.ignoresMouseEvents = false
        if columnPanel.isVisible {
            columnPanel.makeKey()
        }
        self.stage.focusRequest &+= 1
    }

    private func releaseKeyboard() {
        guard let columnPanel, columnPanel.isKeyWindow else { return }
        if NSApp.isActive,
           let other = NSApp.windows.first(where: {
               $0 !== columnPanel && $0 !== self.panel && $0.isVisible && $0.canBecomeKey
           }) {
            other.makeKey()
            return
        }
        columnPanel.orderOut(nil)
        self.orderColumnFront()
    }

    func beginAvatarDrag() {
        self.dragOrigin = self.anchor
    }

    func dragAvatar(by offset: CGSize) {
        guard let origin = self.dragOrigin else { return }
        let candidate = CGPoint(x: origin.x + offset.width, y: origin.y + offset.height)
        let screen = NSScreen.screens.first { $0.frame.contains(candidate) }?.visibleFrame ?? self.screenFrame
        self.anchor = OverlayCanvas.clamp(avatar: candidate, screen: screen)
        self.updatePlacement()
        self.applyAvatar()
        self.applyColumn()
    }

    func endAvatarDrag() {
        guard self.dragOrigin != nil else { return }
        self.dragOrigin = nil
        self.settleAfterMove()
    }

    private func settleAfterMove() {
        guard self.panel != nil else { return }
        self.anchor = OverlayCanvas.clamp(avatar: self.anchor, screen: self.screenFrame)
        self.saveAnchor()
        self.updatePlacement()
        self.applyAvatar()
        self.applyColumn()
        self.evaluatePointer()
    }

    private func updatePlacement() {
        let screen = self.screenFrame
        let placement = OverlayCanvas.place(avatar: self.anchor, screen: screen)
        let limit = OverlayCanvas.columnHeightLimit(avatar: self.anchor, screen: screen, growth: placement.growth)
        if placement != self.stage.placement {
            if self.requested != nil {
                let span = CGSize(width: OverlayCanvas.columnStage.width, height: limit)
                self.requested = OverlayCanvas.columnViewport(size: span, placement: placement)
                self.veilColumn()
            }
            self.stage.placement = placement
        }
        if self.stage.columnHeightLimit != limit {
            self.stage.columnHeightLimit = limit
        }
    }

    private func veilColumn() {
        guard let columnHost else { return }
        columnHost.alphaValue = 0
        Task { @MainActor [weak columnHost] in
            try? await Task.sleep(for: .milliseconds(70))
            guard let columnHost else { return }
            NSAnimationContext.beginGrouping()
            NSAnimationContext.current.duration = BobeMotion.windowFadeSeconds
            columnHost.animator().alphaValue = 1
            NSAnimationContext.endGrouping()
        }
    }

    private func trackPointer(_ tracking: Bool) {
        guard tracking != !self.pointerMonitors.isEmpty else { return }
        guard tracking else {
            for monitor in self.pointerMonitors {
                NSEvent.removeMonitor(monitor)
            }
            self.pointerMonitors.removeAll()
            return
        }
        let mask: NSEvent.EventTypeMask = [.mouseMoved, .leftMouseDragged, .leftMouseUp, .rightMouseUp]
        if let global = NSEvent.addGlobalMonitorForEvents(matching: mask, handler: { [weak self] _ in
            MainActor.assumeIsolated { self?.evaluatePointer() }
        }) {
            self.pointerMonitors.append(global)
        }
        if let local = NSEvent.addLocalMonitorForEvents(matching: mask, handler: { [weak self] event in
            MainActor.assumeIsolated { self?.evaluatePointer() }
            return event
        }) {
            self.pointerMonitors.append(local)
        }
    }

    private func pointerTargets() -> (avatar: Bool, content: Bool) {
        let pointer = NSEvent.mouseLocation
        let avatar = hypot(pointer.x - self.anchor.x, pointer.y - self.anchor.y) <= self.avatarReach
        guard self.requested != nil else { return (avatar, false) }
        let canvas = OverlayCanvas.columnCanvasRect(self.stage.placement)
        let content = self.hitRegions.contains { region in
            OverlayCanvas.windowFrame(for: region.offsetBy(dx: canvas.minX, dy: canvas.minY), avatar: self.anchor)
                .insetBy(dx: -Self.hitMargin, dy: -Self.hitMargin)
                .contains(pointer)
        }
        return (avatar, content)
    }

    private func evaluatePointer() {
        guard let panel, let columnPanel, panel.isVisible else { return }
        let targets = self.pointerTargets()
        if self.stage.pointerOverAvatar != targets.avatar {
            self.stage.pointerOverAvatar = targets.avatar
        }
        if self.stage.pointerOverContent != targets.content {
            self.stage.pointerOverContent = targets.content
        }
        let wantsColumn = self.chatOpen || targets.content
        if targets.avatar, panel.ignoresMouseEvents {
            panel.ignoresMouseEvents = false
        }
        if wantsColumn, columnPanel.ignoresMouseEvents {
            columnPanel.ignoresMouseEvents = false
        }
        let lingering = (!targets.avatar && !panel.ignoresMouseEvents)
            || (!wantsColumn && !columnPanel.ignoresMouseEvents)
        guard lingering else {
            self.releaseTask?.cancel()
            self.releaseTask = nil
            return
        }
        guard self.releaseTask == nil else { return }
        self.releaseTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(Self.releaseMilliseconds))
            guard let self, !Task.isCancelled else { return }
            self.releaseTask = nil
            guard NSEvent.pressedMouseButtons == 0 else { return }
            let targets = self.pointerTargets()
            if !targets.avatar {
                self.panel?.ignoresMouseEvents = true
            }
            if !self.chatOpen, !targets.content {
                self.columnPanel?.ignoresMouseEvents = true
            }
        }
    }

    private func watchClicksAway(_ watching: Bool) {
        guard watching != !self.clickMonitors.isEmpty else { return }
        guard watching else {
            for monitor in self.clickMonitors {
                NSEvent.removeMonitor(monitor)
            }
            self.clickMonitors.removeAll()
            return
        }
        let mask: NSEvent.EventTypeMask = [.leftMouseDown, .rightMouseDown]
        if let global = NSEvent.addGlobalMonitorForEvents(matching: mask, handler: { [weak self] _ in
            MainActor.assumeIsolated { self?.stage.clickAwayCount &+= 1 }
        }) {
            self.clickMonitors.append(global)
        }
        if let local = NSEvent.addLocalMonitorForEvents(matching: mask, handler: { [weak self] event in
            MainActor.assumeIsolated { self?.noteLocalClick(in: event.window) }
            return event
        }) {
            self.clickMonitors.append(local)
        }
    }

    private func noteLocalClick(in window: NSWindow?) {
        guard let window, window !== self.panel else { return }
        if window === self.columnPanel {
            guard !self.pointerTargets().content else { return }
        }
        self.stage.clickAwayCount &+= 1
    }

    private var savedAnchor: CGPoint? {
        let defaults = UserDefaults.standard
        guard defaults.object(forKey: Self.anchorXKey) != nil,
              defaults.object(forKey: Self.anchorYKey) != nil
        else { return nil }
        return CGPoint(
            x: defaults.double(forKey: Self.anchorXKey) - OverlayCanvas.clusterRight,
            y: defaults.double(forKey: Self.anchorYKey) + OverlayCanvas.clusterShort
        )
    }

    private func saveAnchor() {
        let defaults = UserDefaults.standard
        defaults.set(self.anchor.x + OverlayCanvas.clusterRight, forKey: Self.anchorXKey)
        defaults.set(self.anchor.y - OverlayCanvas.clusterShort, forKey: Self.anchorYKey)
    }
}
