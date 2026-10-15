import AppKit
import SwiftUI

enum AvatarMetrics {
    static let ringSize: CGFloat = 80
    static let cardSize: CGFloat = 100
    static let satelliteSize: CGFloat = 30
    static let satelliteOrbit: CGFloat = cardSize / 2 + satelliteSize / 2 - 7
    static let clusterWidth: CGFloat = 132
    static let clusterHeight: CGFloat = 146
    static let center = CGPoint(x: 72, y: 86)
    static let restAngle: Double = 38
    static let chatAngle: Double = 222
    static let micAngle: Double = 264
    static let menuAngle: Double = 306
    static let pipAngle: Double = 184
    static let toolAngle: Double = 140
    static let sleepDelaySeconds: TimeInterval = 75
    static let celebrationSeconds: TimeInterval = 1.25
    static let satelliteLingerMilliseconds = 450
}

struct OverlayAvatar: View {
    let mood: BobeMood
    let isEngaged: Bool
    let wantsAttention: Bool
    let celebrationToken: Int
    let onTap: () -> Void
    let onSecondaryTap: (NSEvent, NSView) -> Void

    @Environment(\.theme) private var theme
    @State private var isPressed = false
    @State private var isHovered = false
    @State private var pipeline = VoicePipeline.shared
    @State private var pointer = PointerTracker.shared
    @State private var isAsleep = false
    @State private var isCelebrating = false
    @State private var isTracking = false
    @State private var speechBeat = 0.0

    private var displayMood: BobeMood {
        guard self.mood == .idle else { return self.mood }
        if self.isCelebrating { return .happy }
        return self.isAsleep && !self.isEngaged ? .asleep : .idle
    }

    private var gaze: CGSize {
        guard self.isTracking, OverlayWindowManager.shared.panel != nil else { return .zero }
        return PointerTracker.gaze(from: OverlayWindowManager.shared.anchor, to: self.pointer.location)
    }

    private var voiceLevel: Double {
        switch self.displayMood {
        case .listening: Double(self.pipeline.inputLevel)
        case .speaking: max(Double(self.pipeline.ttsOutputLevel), self.speechBeat * 0.35)
        default: 0
        }
    }

    private var mouthLevel: Double {
        guard self.displayMood == .speaking else { return 0 }
        return min(1, self.speechBeat * 0.2 + Double(self.pipeline.ttsOutputLevel) * 1.2)
    }

    var body: some View {
        BobeCharacter(
            mood: self.displayMood,
            size: AvatarMetrics.ringSize,
            palette: self.theme.avatar,
            gaze: self.gaze,
            voiceLevel: self.voiceLevel,
            mouthLevel: self.mouthLevel,
            wantsAttention: self.wantsAttention
        )
        .frame(width: AvatarMetrics.cardSize, height: AvatarMetrics.cardSize)
        .bobeRaised(in: Circle(), elevation: .lifted, fill: self.theme.colors.background)
        .overlay(alignment: .topTrailing) {
            if self.displayMood == .asleep {
                BobeSleepNotes(color: self.theme.avatar.orbit)
                    .offset(x: 6, y: -10)
                    .transition(.opacity)
            }
        }
        .overlay {
            if self.displayMood == .thinking || self.displayMood == .working, !BobeMotion.isReduced {
                BobeWorkingRim(shape: Circle(), lineWidth: 2)
                    .transition(.opacity)
            }
        }
        .animation(BobeMotion.soft, value: self.displayMood)
        .scaleEffect(self.isPressed ? 0.95 : (self.isHovered ? 1.03 : 1))
        .animation(BobeMotion.body, value: self.isPressed)
        .animation(BobeMotion.standard, value: self.isHovered)
        .overlay {
            AvatarPressSurface(
                onClick: self.onTap,
                onSecondaryClick: self.onSecondaryTap,
                onPressChange: { self.isPressed = $0 }
            )
            .frame(width: AvatarMetrics.cardSize, height: AvatarMetrics.cardSize)
        }
        .onHover { self.isHovered = $0 }
        .onChange(of: self.displayMood.followsPointer, initial: true) { _, follows in
            self.setTracking(follows)
        }
        .onDisappear { self.setTracking(false) }
        .task(id: "\(self.mood)|\(self.isEngaged)") {
            if self.mood != .idle || self.isEngaged {
                self.isAsleep = false
                return
            }
            try? await Task.sleep(for: .seconds(AvatarMetrics.sleepDelaySeconds))
            guard !Task.isCancelled else { return }
            self.isAsleep = true
        }
        .onChange(of: self.celebrationToken) { _, _ in
            self.isCelebrating = true
            Task { @MainActor in
                try? await Task.sleep(for: .seconds(AvatarMetrics.celebrationSeconds))
                self.isCelebrating = false
            }
        }
        .task(id: self.displayMood == .speaking) {
            self.speechBeat = 0
            guard self.displayMood == .speaking, !BobeMotion.isReduced else { return }
            let beats: [Double] = [0.7, 0.3, 1.0, 0.5, 0.85, 0.2, 0.6, 0.95]
            var index = 0
            while !Task.isCancelled {
                self.speechBeat = beats[index % beats.count]
                index &+= 1
                try? await Task.sleep(for: .milliseconds(180))
            }
        }
        .accessibilityElement()
        .accessibilityLabel(L10n.tr("overlay.avatar.accessibility_format", self.stateAccessibilityText))
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { self.onTap() }
        .accessibilityIdentifier("overlay.avatar")
    }

    private func setTracking(_ tracking: Bool) {
        guard tracking != self.isTracking else { return }
        self.isTracking = tracking
        if tracking {
            self.pointer.retain()
        } else {
            self.pointer.release()
        }
    }

    private var stateAccessibilityText: String {
        switch self.displayMood {
        case .waking: L10n.tr("overlay.avatar.state.loading")
        case .worried: L10n.tr("overlay.avatar.state.error")
        case .asleep, .idle, .happy: L10n.tr("overlay.avatar.state.idle")
        case .watching: L10n.tr("overlay.avatar.state.watching")
        case .thinking, .working: L10n.tr("overlay.avatar.state.thinking")
        case .speaking: L10n.tr("overlay.avatar.state.speaking")
        case .eager: L10n.tr("overlay.avatar.state.wants_to_speak")
        case .listening: L10n.tr("overlay.voice.presence_ring.accessibility")
        }
    }
}

struct BobeSleepNotes: View {
    let color: Color
    @State private var cycle = 0

    var body: some View {
        ZStack {
            ForEach(0 ..< 3, id: \.self) { index in
                SleepNote(color: self.color, index: index, cycle: self.cycle)
            }
        }
        .frame(width: 34, height: 40)
        .task {
            guard !BobeMotion.isReduced else { return }
            while !Task.isCancelled {
                self.cycle &+= 1
                try? await Task.sleep(for: .seconds(9))
            }
        }
        .accessibilityHidden(true)
    }
}

private struct SleepNote: View {
    let color: Color
    let index: Int
    let cycle: Int
    @State private var progress = 0.0

    var body: some View {
        Text(verbatim: "z")
            .font(.system(size: 9 + CGFloat(self.index) * 2.5, weight: .bold, design: .rounded))
            .foregroundStyle(self.color)
            .offset(
                x: -8 + self.progress * (10 + CGFloat(self.index) * 5),
                y: 14 - self.progress * (18 + CGFloat(self.index) * 6)
            )
            .opacity(sin(.pi * self.progress) * 0.9)
            .onChange(of: self.cycle) { _, _ in
                var transaction = Transaction()
                transaction.disablesAnimations = true
                withTransaction(transaction) { self.progress = 0 }
                withAnimation(BobeMotion.drift(self.index)) {
                    self.progress = 1
                }
            }
    }
}

@Animatable
struct OrbitPlacement: ViewModifier {
    var angle: Double
    var radius: CGFloat

    func body(content: Content) -> some View {
        let radians = self.angle * .pi / 180
        content.offset(x: self.radius * cos(radians), y: self.radius * sin(radians))
    }
}

struct OrbitSlot<Content: View>: View {
    let angle: Double
    let isShown: Bool
    var index = 0
    @ViewBuilder let content: Content

    var body: some View {
        self.content
            .scaleEffect(self.isShown ? 1 : 0.35)
            .opacity(self.isShown ? 1 : 0)
            .modifier(
                OrbitPlacement(
                    angle: self.isShown ? self.angle : self.angle - 70,
                    radius: self.isShown ? AvatarMetrics.satelliteOrbit : AvatarMetrics.satelliteOrbit - 22
                )
            )
            .allowsHitTesting(self.isShown)
            .animation(BobeMotion.stagger(self.index, step: 0.045), value: self.isShown)
    }
}

struct SatelliteChrome: View {
    let systemName: String
    var isActive = false
    var isDimmed = false
    @Environment(\.theme) private var theme
    @State private var isHovered = false

    var body: some View {
        Image(systemName: self.systemName)
            .font(.system(size: 12.5, weight: .semibold))
            .foregroundStyle(self.foreground)
            .opacity(self.isDimmed ? 0.5 : 1)
            .contentTransition(BobeMotion.symbolReplace)
            .frame(width: AvatarMetrics.satelliteSize, height: AvatarMetrics.satelliteSize)
            .bobeRaised(in: Circle(), fill: self.isActive ? self.theme.colors.primary : nil)
            .scaleEffect(self.isHovered ? 1.1 : 1)
            .animation(BobeMotion.snappy, value: self.isHovered)
            .animation(BobeMotion.standard, value: self.isActive)
            .onHover { self.isHovered = $0 }
            .contentShape(Circle())
    }

    private var foreground: Color {
        if self.isActive { return self.theme.onAccent }
        return self.isHovered ? self.theme.colors.text : self.theme.colors.textMuted
    }
}

struct ChatToggleButton: View {
    var isActive = false
    var hasUnread = false
    let action: () -> Void
    @Environment(\.theme) private var theme

    var body: some View {
        Button(action: self.action) {
            SatelliteChrome(systemName: self.isActive ? "xmark" : "bubble.left.fill", isActive: self.isActive)
                .overlay(alignment: .topTrailing) {
                    if self.hasUnread, !self.isActive {
                        Circle()
                            .fill(self.theme.colors.primary)
                            .frame(width: 9, height: 9)
                            .overlay { Circle().strokeBorder(self.theme.colors.surface, lineWidth: 1.5) }
                            .transition(.bobePop())
                    }
                }
        }
        .buttonStyle(BobePressStyle(scale: 0.9))
        .accessibilityLabel(
            self.isActive
                ? L10n.tr("overlay.chat_toggle.hide.accessibility")
                : L10n.tr("overlay.chat_toggle.show.accessibility")
        )
        .accessibilityValue(self.hasUnread ? L10n.tr("overlay.app_menu.unread") : "")
        .accessibilityIdentifier("overlay.avatar.chat-toggle")
    }
}

struct AvatarPressSurface: NSViewRepresentable {
    let onClick: () -> Void
    let onSecondaryClick: (NSEvent, NSView) -> Void
    let onPressChange: (Bool) -> Void

    func makeNSView(context: Context) -> AvatarPressView {
        let view = AvatarPressView()
        self.updateNSView(view, context: context)
        return view
    }

    func updateNSView(_ view: AvatarPressView, context: Context) {
        view.onClick = self.onClick
        view.onSecondaryClick = self.onSecondaryClick
        view.onPressChange = self.onPressChange
    }
}

final class AvatarPressView: NSView {
    var onClick: (() -> Void)?
    var onSecondaryClick: ((NSEvent, NSView) -> Void)?
    var onPressChange: ((Bool) -> Void)?
    private var pressOrigin: NSPoint?
    private var didDrag = false

    private static let dragThreshold: CGFloat = 3

    override var mouseDownCanMoveWindow: Bool {
        false
    }

    override func acceptsFirstMouse(for event: NSEvent?) -> Bool {
        true
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        guard let superview else { return nil }
        let local = self.convert(point, from: superview)
        let radius = min(self.bounds.width, self.bounds.height) / 2
        let distance = hypot(local.x - self.bounds.midX, local.y - self.bounds.midY)
        return distance <= radius ? self : nil
    }

    override func mouseDown(with event: NSEvent) {
        self.pressOrigin = NSEvent.mouseLocation
        self.didDrag = false
        self.onPressChange?(true)
        OverlayWindowManager.shared.beginAvatarDrag()
    }

    override func mouseDragged(with event: NSEvent) {
        guard let origin = self.pressOrigin else { return }
        let current = NSEvent.mouseLocation
        let travel = CGSize(width: current.x - origin.x, height: current.y - origin.y)
        if !self.didDrag {
            guard hypot(travel.width, travel.height) > Self.dragThreshold else { return }
            self.didDrag = true
            self.onPressChange?(false)
        }
        OverlayWindowManager.shared.dragAvatar(by: travel)
    }

    override func mouseUp(with event: NSEvent) {
        defer { self.pressOrigin = nil }
        self.onPressChange?(false)
        OverlayWindowManager.shared.endAvatarDrag()
        guard self.pressOrigin != nil, !self.didDrag else { return }
        self.onClick?()
    }

    override func rightMouseDown(with event: NSEvent) {
        self.onSecondaryClick?(event, self)
    }
}

struct ReachoutPip: View {
    let count: Int

    @Environment(\.theme) private var theme

    var body: some View {
        Text(self.count > 9 ? "9+" : "\(self.count)")
            .font(.system(size: 10.5, weight: .bold, design: .rounded))
            .monospacedDigit()
            .contentTransition(.numericText())
            .foregroundStyle(self.theme.onAccent)
            .padding(.horizontal, 5)
            .frame(minWidth: 18, minHeight: 18)
            .background(Capsule(style: .circular).fill(self.theme.colors.primary))
            .overlay(Capsule(style: .circular).strokeBorder(self.theme.colors.surface, lineWidth: 1.5))
            .shadow(color: self.theme.shadowTint.opacity(0.22), radius: 3, x: 0, y: 1.5)
            .accessibilityLabel(L10n.tr("overlay.reachout.waiting_format", self.count))
    }
}

enum ToolGlyph {
    private static let table: [(keys: [String], symbol: String)] = [
        (["search", "grep", "find", "glob", "lookup"], "magnifyingglass"),
        (["bash", "shell", "exec", "command", "terminal", "run"], "terminal.fill"),
        (["web", "fetch", "http", "browse", "url"], "globe"),
        (["write", "edit", "patch", "create"], "pencil"),
        (["read", "file", "list", "ls", "open"], "doc.text.fill"),
        (["memory", "recall", "remember"], "brain.head.profile"),
        (["goal"], "flag.fill"),
        (["schedule", "remind", "timer", "clock"], "clock.fill"),
        (["delegate", "agent", "task", "job"], "person.2.fill"),
        (["skill"], "wand.and.stars"),
        (["git"], "arrow.triangle.branch"),
        (["screen", "capture", "vision", "image"], "viewfinder"),
    ]

    static func symbol(for toolName: String) -> String {
        let name = toolName.lowercased()
        return self.table.first { entry in entry.keys.contains { name.contains($0) } }?.symbol
            ?? "wrench.and.screwdriver.fill"
    }
}

struct ToolBadge: View {
    let toolName: String
    let count: Int

    @Environment(\.theme) private var theme

    private static let side: CGFloat = 26

    var body: some View {
        Image(systemName: ToolGlyph.symbol(for: self.toolName))
            .font(.system(size: 11, weight: .semibold))
            .foregroundStyle(self.theme.colors.primary)
            .contentTransition(BobeMotion.symbolReplace)
            .frame(width: Self.side, height: Self.side)
            .bobeRaised(in: Circle())
            .overlay {
                BobeSpinner(size: Self.side - 3, lineWidth: 1.6, color: self.theme.glow[1])
            }
            .overlay(alignment: .topTrailing) {
                if self.count > 1 {
                    Text(verbatim: "\(min(self.count, 9))")
                        .font(.system(size: 8.5, weight: .bold, design: .rounded))
                        .foregroundStyle(self.theme.onAccent)
                        .frame(width: 12, height: 12)
                        .background(Circle().fill(self.theme.colors.primary))
                        .offset(x: 3, y: -3)
                        .transition(.bobePop())
                }
            }
            .animation(BobeMotion.snappy, value: self.count)
            .accessibilityElement()
            .accessibilityLabel(L10n.tr("overlay.status.using_tool_format", ToolChip.displayName(self.toolName)))
    }
}
