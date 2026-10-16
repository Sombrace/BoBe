import SwiftUI

struct AvatarStatusBubble: View {
    enum Mode {
        case hidden
        case bobeMessage(ChatMessage)
        case userTranscribing(text: String)
        case status(kind: StatusKind)
        case silenced

        enum StatusKind: Equatable {
            case thinking
            case toolRunning(name: String)
            case reconnecting
            case starting

            @MainActor var label: String {
                switch self {
                case .thinking: L10n.tr("overlay.status.thinking")
                case let .toolRunning(name):
                    L10n.tr("overlay.status.using_tool_format", ToolChip.displayName(name))
                case .reconnecting: L10n.tr("overlay.reconnecting")
                case .starting: L10n.tr("overlay.status.starting")
                }
            }
        }
    }

    let mode: Mode
    let onOpenAnswer: () -> Void
    let onToggleSilence: () -> Void
    let onDismiss: () -> Void
    let isSilenced: Bool
    let isTtsAudible: Bool
    let dismissalStyle: AmbientBubbleDismissalStyle
    let recedeProgress: Double
    let onInteractionChanged: (Bool) -> Void
    var onStop: (() -> Void)?
    var reachout: ReachoutKind = .answer
    var onReply: () -> Void = {}
    var onTellMore: () -> Void = {}
    var onSettle: () -> Void = {}

    @Environment(\.theme) private var theme
    @Environment(\.overlayPlacement) private var placement
    @State private var isHovered = false
    @State private var messageContentHeight: CGFloat = 20
    @State private var messageScrolledToBottom = false
    @FocusState private var focusedAction: BubbleAction?
    @AccessibilityFocusState private var accessibilityFocus: BubbleAction?

    private static let transcriptBottomAnchor = "ambient-transcript-bottom"

    private enum BubbleAction: Hashable {
        case message
        case openAnswer
        case silence
        case dismiss
    }

    private var bubbleShape: UnevenRoundedRectangle {
        self.placement.bubbleShape()
    }

    var body: some View {
        Group {
            switch self.mode {
            case .hidden:
                EmptyView()
            case let .bobeMessage(message):
                self.messageBubble(message)
            case let .userTranscribing(text):
                self.transcriptBubble(text)
            case let .status(kind):
                self.statusPill(kind)
            case .silenced:
                self.silencedPill
            }
        }
        .overlayHitRegion()
        .frame(maxWidth: .infinity, alignment: self.placement.nearEdgeAlignment)
        .modifier(
            AmbientBubbleTransition(
                style: BobeMotion.isReduced ? .fade : self.dismissalStyle,
                progress: self.recedeProgress
            )
        )
        .onChange(of: self.focusedAction) { _, _ in self.reportInteraction() }
        .onChange(of: self.accessibilityFocus) { _, _ in self.reportInteraction() }
    }

    private func statusPill(_ kind: Mode.StatusKind) -> some View {
        HStack(spacing: 8) {
            BobeBreathingDot()
            BobeShimmerText(text: kind.label)
                .id(kind.label)
                .transition(.bobeRise)
            if let onStop {
                StopButton(action: onStop)
                    .transition(.bobePop())
            }
        }
        .padding(.leading, 13)
        .padding(.trailing, self.onStop == nil ? 14 : 7)
        .frame(height: 34)
        .bobeRaised(in: .bobePill)
        .animation(BobeMotion.snappy, value: kind)
        .fixedSize()
        .accessibilityElement(children: .contain)
    }

    private var silencedPill: some View {
        Button(action: self.onToggleSilence) {
            HStack(spacing: 7) {
                Image(systemName: "moon.zzz.fill")
                    .font(.system(size: 11, weight: .semibold))
                Text(L10n.tr("overlay.bubble.sender.silenced"))
                    .font(.system(size: 12.5, weight: .medium))
            }
            .foregroundStyle(self.theme.colors.textMuted)
            .padding(.horizontal, 13)
            .frame(height: 32)
            .bobeRaised(in: .bobePill)
        }
        .buttonStyle(BobePressStyle())
        .fixedSize()
        .help(L10n.tr("overlay.floating_bubble.unsilence"))
    }

    private func transcriptBubble(_ text: String) -> some View {
        ScrollViewReader { proxy in
            ScrollView(.vertical, showsIndicators: false) {
                HStack(alignment: .firstTextBaseline, spacing: 7) {
                    Image(systemName: "waveform")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(self.theme.colors.primary)
                        .symbolEffect(.variableColor.iterative, isActive: !BobeMotion.isReduced)
                    Text(text)
                        .bobeTextStyle(.chatBody)
                        .lineSpacing(2)
                        .foregroundStyle(self.theme.colors.text)
                        .multilineTextAlignment(.leading)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Color.clear
                    .frame(height: 1)
                    .id(Self.transcriptBottomAnchor)
            }
            .frame(maxHeight: 120)
            .scrollBounceBehavior(.basedOnSize)
            .defaultScrollAnchor(.bottom)
            .onChange(of: text) { _, _ in
                proxy.scrollTo(Self.transcriptBottomAnchor, anchor: .bottom)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .padding(.horizontal, 13)
        .padding(.vertical, 9)
        .bobeRaised(in: self.bubbleShape, fill: self.theme.accentSoft)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(L10n.tr("overlay.bubble.sender.you")): \(text)")
    }

    private func messageBubble(_ message: ChatMessage) -> some View {
        let limit = WindowSizes.heightAmbientMessageViewport
        let overflows = self.messageContentHeight > limit + 1
        let viewportHeight = min(max(20, self.messageContentHeight), limit)
        let fadeStart = max(0, 1 - (22 / max(viewportHeight, 1)))
        let showsActions = self.isHovered || self.focusedAction != nil || self.accessibilityFocus != nil
        let text = ScrollView(.vertical, showsIndicators: overflows) {
            VStack(alignment: .leading, spacing: 6) {
                if message.isStreaming {
                    StreamingText(content: message.content)
                } else {
                    CompletedReplyText(markdown: message.content)
                }
                if !message.isComplete, !message.isStreaming {
                    Label(L10n.tr("overlay.chat.interrupted"), systemImage: "exclamationmark.circle")
                        .bobeTextStyle(.chatPending)
                        .foregroundStyle(self.theme.colors.error)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .onGeometryChange(for: CGFloat.self) { geometry in
                ceil(geometry.size.height)
            } action: { height in
                if height > 0 { self.messageContentHeight = height }
            }
        }
        .frame(height: viewportHeight)
        .scrollDisabled(!overflows)
        .scrollBounceBehavior(.basedOnSize)
        .textSelection(.enabled)
        .onScrollGeometryChange(for: Bool.self) { geometry in
            geometry.visibleRect.maxY >= geometry.contentSize.height - 1
        } action: { _, isAtBottom in
            self.messageScrolledToBottom = isAtBottom
        }
        .mask {
            LinearGradient(
                stops: [
                    .init(color: .black, location: 0),
                    .init(color: .black, location: overflows && !self.messageScrolledToBottom ? fadeStart : 1),
                    .init(color: overflows && !self.messageScrolledToBottom ? .clear : .black, location: 1),
                ],
                startPoint: .top,
                endPoint: .bottom
            )
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(
            "\(L10n.tr("overlay.chat.sender.bobe")): \(CompletedReplyText.plainText(fromMarkdown: message.content))"
        )
        .accessibilityValue(self.messageAccessibilityValue(message))
        .accessibilityFocused(self.$accessibilityFocus, equals: .message)
        let offersReplies = message.isComplete && !message.isStreaming && self.reachout != .answer
        return VStack(alignment: .leading, spacing: 9) {
            if let label = self.reachout.label, let symbol = self.reachout.symbol {
                HStack(spacing: 5) {
                    Image(systemName: symbol)
                        .font(.system(size: 9.5, weight: .bold))
                    Text(label)
                        .bobeTextStyle(.caption)
                }
                .foregroundStyle(self.theme.colors.primary)
                .transition(.bobeRise)
                .accessibilityHidden(true)
            }
            text
            if offersReplies {
                self.quickReplies
                    .transition(.bobeRise)
            }
        }
        .animation(BobeMotion.standard, value: offersReplies)
        .animation(BobeMotion.standard, value: self.reachout)
        .padding(.horizontal, 14)
        .padding(.vertical, 11)
        .bobeRaised(in: self.bubbleShape, elevation: .lifted)
        .overlay(alignment: .topTrailing) {
            self.actionCluster(overflows: overflows)
                .offset(x: -8, y: -15)
                .opacity(showsActions ? 1 : 0)
                .scaleEffect(showsActions ? 1 : 0.85, anchor: .bottomTrailing)
                .animation(BobeMotion.snappy, value: showsActions)
        }
        .padding(.top, 16)
        .contentShape(Rectangle())
        .onHover { hovering in
            self.isHovered = hovering
            self.reportInteraction()
        }
        .onChange(of: message.id) { _, _ in
            self.messageContentHeight = 20
            self.messageScrolledToBottom = false
        }
    }

    private var quickReplies: some View {
        ViewThatFits(in: .horizontal) {
            self.quickReplyRow(compact: false)
            self.quickReplyRow(compact: true)
        }
    }

    private func quickReplyRow(compact: Bool) -> some View {
        HStack(spacing: 6) {
            switch self.reachout {
            case .thought:
                self.replyChip(
                    L10n.tr("overlay.reachout.action.reply"),
                    systemName: "arrowshape.turn.up.left.fill",
                    isProminent: true, iconOnly: compact, id: "reply", action: self.onReply
                )
                self.replyChip(L10n.tr("overlay.reachout.action.more"), id: "more", action: self.onTellMore)
                self.replyChip(L10n.tr("overlay.reachout.action.later"), id: "later", action: self.onSettle)
            case .report:
                self.replyChip(
                    L10n.tr("overlay.reachout.action.open"),
                    systemName: "arrow.up.left.and.arrow.down.right",
                    isProminent: true, iconOnly: compact, id: "open", action: self.onOpenAnswer
                )
                self.replyChip(L10n.tr("overlay.reachout.action.done"), id: "done", action: self.onSettle)
            case .answer:
                EmptyView()
            }
        }
        .fixedSize()
    }

    private func replyChip(
        _ title: String,
        systemName: String? = nil,
        isProminent: Bool = false,
        iconOnly: Bool = false,
        id: String,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            HStack(spacing: 5) {
                if let systemName {
                    Image(systemName: systemName)
                        .font(.system(size: 9.5, weight: .bold))
                }
                if !(iconOnly && systemName != nil) {
                    Text(title)
                        .bobeTextStyle(.caption)
                        .lineLimit(1)
                }
            }
            .foregroundStyle(isProminent ? self.theme.onAccent : self.theme.colors.text)
            .padding(.horizontal, 11)
            .frame(height: 26)
            .background(
                Capsule(style: .circular)
                    .fill(isProminent ? self.theme.colors.primary : self.theme.well)
            )
            .contentShape(Capsule(style: .circular))
        }
        .buttonStyle(BobePressStyle(scale: 0.94))
        .accessibilityLabel(title)
        .accessibilityIdentifier("overlay.bubble.\(id)")
    }

    private func messageAccessibilityValue(_ message: ChatMessage) -> String {
        if message.isStreaming { return L10n.tr("overlay.status.thinking") }
        return message.isComplete ? "" : L10n.tr("overlay.chat.interrupted")
    }

    private func actionCluster(overflows: Bool) -> some View {
        HStack(spacing: 2) {
            self.actionButton(
                systemName: "arrow.up.left.and.arrow.down.right",
                label: L10n.tr("overlay.floating_bubble.open_full_answer"),
                focus: .openAnswer,
                action: self.onOpenAnswer
            )
            .accessibilityIdentifier("overlay.bubble.open-full-answer")
            if self.isTtsAudible || self.isSilenced {
                self.actionButton(
                    systemName: self.isSilenced ? "speaker.slash.fill" : "speaker.wave.2.fill",
                    label: L10n.tr(
                        self.isSilenced ? "overlay.floating_bubble.unsilence" : "overlay.floating_bubble.silence"
                    ),
                    focus: .silence,
                    action: self.onToggleSilence
                )
            }
            self.actionButton(
                systemName: "xmark",
                label: L10n.tr("overlay.floating_bubble.dismiss"),
                focus: .dismiss,
                action: self.onDismiss
            )
        }
        .padding(3)
        .bobeRaised(in: .bobePill)
    }

    private func actionButton(
        systemName: String,
        label: String,
        focus: BubbleAction,
        action: @escaping () -> Void
    ) -> some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(self.theme.colors.textMuted)
                .frame(width: 24, height: 22)
                .contentShape(Capsule())
        }
        .buttonStyle(BobePressStyle(scale: 0.88))
        .focused(self.$focusedAction, equals: focus)
        .accessibilityFocused(self.$accessibilityFocus, equals: focus)
        .accessibilityLabel(label)
        .help(label)
    }

    private func reportInteraction() {
        self.onInteractionChanged(
            self.isHovered || self.focusedAction != nil || self.accessibilityFocus != nil
        )
    }
}
