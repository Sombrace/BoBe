import AppKit
import SwiftUI
import Textual

private struct ThreadScrollState: Equatable {
    var top: CGFloat
    var bottom: CGFloat
    var content: CGFloat
    var container: CGFloat
}

struct ConversationSheet: View {
    let earlier: [ChatMessage]
    let current: [ChatMessage]
    let isExpanded: Bool
    let onToggleExpanded: () -> Void
    let activity: String?
    let tools: [ToolExecution]
    var maxViewportHeight: CGFloat = WindowSizes.heightChatViewportMax
    var onPresented: @MainActor (String) async -> Void = { _ in }

    @State private var contentHeight: CGFloat = 0
    @State private var rowHeights: [String: CGFloat] = [:]
    @State private var activityHeight: CGFloat = 0
    @State private var isAtTop = true
    @State private var isAtBottom = true
    @State private var followsLatest = true
    @State private var holdsBottomUntil: Date = .distantPast
    @State private var ignoresScrollUntil: Date = .distantPast
    @State private var settlesUntil: Date = .distantPast
    @Environment(\.theme) private var theme
    @Environment(\.overlayPlacement) private var placement

    private static let bottomAnchor = "conversation-bottom"
    private static let rowSpacing: CGFloat = 12
    private static let bottomMarker: CGFloat = 0.5
    private static let compactEarlierWindow = 10
    private static let earlierPeek: CGFloat = 14
    private static let nearBottomSlack: CGFloat = 48
    private static let fadeHeight: CGFloat = 16

    private var shownEarlier: [ChatMessage] {
        self.isExpanded ? self.earlier : Array(self.earlier.suffix(Self.compactEarlierWindow))
    }

    private var showsActivity: Bool {
        self.activity != nil || !self.tools.isEmpty
    }

    private func height(of messages: some Collection<ChatMessage>) -> CGFloat {
        let rows = messages.reduce(CGFloat(0)) { $0 + (self.rowHeights[$1.id] ?? 0) }
        let items = messages.count + (self.showsActivity ? 1 : 0)
        return rows + (self.showsActivity ? self.activityHeight : 0)
            + CGFloat(items) * Self.rowSpacing + Self.bottomMarker
    }

    private var question: ChatMessage? {
        self.current.last { $0.sender == .user }
    }

    private var answerHeight: CGFloat {
        guard let question, let index = self.current.firstIndex(where: { $0.id == question.id }) else {
            return self.height(of: self.current)
        }
        return self.height(of: self.current[index...])
    }

    private var restingHeight: CGFloat {
        self.isExpanded || self.shownEarlier.isEmpty
            ? self.contentHeight
            : self.height(of: self.current) + Self.earlierPeek
    }

    private var viewportHeight: CGFloat {
        min(max(24, self.restingHeight), self.maxViewportHeight)
    }

    private var showsHandle: Bool {
        !self.earlier.isEmpty || self.isExpanded
    }

    private var sizeAnchor: UnitPoint {
        if self.followsLatest { return .bottom }
        return self.placement.growth == .up && Date.now < self.holdsBottomUntil ? .bottom : .top
    }

    var body: some View {
        VStack(spacing: 0) {
            if self.showsHandle {
                self.historyHandle
            }
            self.thread
        }
        .padding(.horizontal, 14)
        .padding(.top, self.showsHandle ? 4 : 12)
        .padding(.bottom, 12)
        .bobeRaised(cornerRadius: BobeRadius.card, elevation: .lifted)
    }

    private var historyHandle: some View {
        Button(action: self.onToggleExpanded) {
            Image(systemName: self.isExpanded ? "chevron.compact.down" : "chevron.compact.up")
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(self.theme.colors.textMuted.opacity(0.75))
                .frame(maxWidth: .infinity)
                .frame(height: 20)
                .contentShape(Rectangle())
        }
        .buttonStyle(BobePressStyle())
        .accessibilityLabel(self.handleLabel)
        .accessibilityIdentifier("overlay.composer.history")
        .help(self.handleLabel)
    }

    private var handleLabel: String {
        L10n.tr(
            self.isExpanded
                ? "overlay.input.history.hide.accessibility"
                : "overlay.input.history.show.accessibility"
        )
    }

    private var thread: some View {
        ScrollViewReader { proxy in
            ScrollView(showsIndicators: self.contentHeight > self.viewportHeight + 1) {
                VStack(alignment: .leading, spacing: Self.rowSpacing) {
                    ForEach(self.shownEarlier + self.current) { message in
                        MessageRow(message: message, onPresented: self.onPresented)
                            .onGeometryChange(for: CGFloat.self) { geometry in
                                ceil(geometry.size.height)
                            } action: { height in
                                self.rowHeights[message.id] = height
                            }
                            .id(message.id)
                            .transition(.bobeRise)
                    }
                    if self.showsActivity {
                        LiveActivityLine(activity: self.activity, tools: self.tools)
                            .onGeometryChange(for: CGFloat.self) { geometry in
                                ceil(geometry.size.height)
                            } action: { height in
                                self.activityHeight = height
                            }
                            .transition(.bobeRise)
                    }
                    Color.clear
                        .frame(height: Self.bottomMarker)
                        .id(Self.bottomAnchor)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .animation(BobeMotion.standard, value: self.current.last?.id)
                .animation(BobeMotion.standard, value: self.showsActivity)
                .onGeometryChange(for: CGFloat.self) { geometry in
                    ceil(geometry.size.height)
                } action: { height in
                    self.contentHeight = height
                }
            }
            .frame(height: self.viewportHeight)
            .animation(BobeMotion.standard, value: self.viewportHeight)
            .scrollBounceBehavior(.basedOnSize)
            .defaultScrollAnchor(.bottom)
            .defaultScrollAnchor(self.sizeAnchor, for: .sizeChanges)
            .accessibilityIdentifier("overlay.chat.scroll")
            .onScrollGeometryChange(for: ThreadScrollState.self) { geometry in
                ThreadScrollState(
                    top: geometry.visibleRect.minY,
                    bottom: geometry.visibleRect.maxY,
                    content: geometry.contentSize.height,
                    container: geometry.containerSize.height
                )
            } action: { old, new in
                self.scrolled(from: old, to: new)
            }
            .mask { self.edgeFade }
            .overlay(alignment: .bottom) {
                if !self.followsLatest, !self.isAtBottom {
                    self.latestButton(proxy)
                        .transition(.bobePop(anchor: .bottom))
                }
            }
            .animation(BobeMotion.snappy, value: self.followsLatest || self.isAtBottom)
            .onAppear {
                self.settlesUntil = .now.addingTimeInterval(1)
                self.position(proxy, animated: false)
            }
            .onChange(of: self.contentHeight) { _, _ in
                if Date.now < self.settlesUntil {
                    self.position(proxy, animated: false)
                }
            }
            .onChange(of: self.current.last?.id) { _, _ in
                let known = Set((self.earlier + self.current).map(\.id))
                self.rowHeights = self.rowHeights.filter { known.contains($0.key) }
                if self.current.last?.sender == .user {
                    self.followsLatest = true
                }
                self.follow(proxy)
            }
            .onChange(of: self.current.last?.content) { _, _ in
                self.follow(proxy)
            }
            .onChange(of: self.activity) { _, _ in
                self.follow(proxy)
            }
            .onChange(of: self.isExpanded) { _, expanded in
                self.holdsBottomUntil = .now.addingTimeInterval(0.8)
                self.ignoresScrollUntil = .now.addingTimeInterval(0.8)
                if !expanded {
                    self.jumpToLatest(proxy, animated: false)
                }
            }
        }
    }

    private var edgeFade: some View {
        let fade = min(0.5, Self.fadeHeight / max(self.viewportHeight, 1))
        return LinearGradient(
            stops: [
                .init(color: .black.opacity(self.isAtTop ? 1 : 0), location: 0),
                .init(color: .black, location: fade),
                .init(color: .black, location: 1 - fade),
                .init(color: .black.opacity(self.isAtBottom ? 1 : 0), location: 1),
            ],
            startPoint: .top,
            endPoint: .bottom
        )
    }

    private func latestButton(_ proxy: ScrollViewProxy) -> some View {
        Button {
            self.jumpToLatest(proxy, animated: true)
        } label: {
            Image(systemName: "arrow.down")
                .font(.system(size: 11, weight: .bold))
                .foregroundStyle(self.theme.colors.text)
                .frame(width: 28, height: 28)
                .bobeRaised(in: Circle(), elevation: .lifted)
        }
        .buttonStyle(BobePressStyle(scale: 0.9))
        .padding(.bottom, 6)
        .accessibilityLabel(L10n.tr("overlay.chat.jump_latest"))
        .accessibilityIdentifier("overlay.chat.jump-latest")
        .help(L10n.tr("overlay.chat.jump_latest"))
    }

    private func scrolled(from old: ThreadScrollState, to new: ThreadScrollState) {
        self.isAtTop = new.top <= 1
        self.isAtBottom = new.bottom >= new.content - 1
        guard Date.now >= self.ignoresScrollUntil,
              abs(new.content - old.content) < 0.5,
              abs(new.container - old.container) < 0.5
        else { return }
        if new.bottom >= new.content - Self.nearBottomSlack {
            if new.top > old.top + 0.5 {
                self.followsLatest = true
            }
        } else if new.top < old.top - 0.5 {
            self.followsLatest = false
            if !self.isExpanded, !self.earlier.isEmpty {
                self.onToggleExpanded()
            }
        }
    }

    private func jumpToLatest(_ proxy: ScrollViewProxy, animated: Bool) {
        self.followsLatest = true
        self.ignoresScrollUntil = .now.addingTimeInterval(0.4)
        withAnimation(animated ? BobeMotion.standard : nil) {
            proxy.scrollTo(Self.bottomAnchor, anchor: .bottom)
        }
    }

    private func follow(_ proxy: ScrollViewProxy) {
        guard self.followsLatest else { return }
        self.position(proxy, animated: true)
    }

    private func position(_ proxy: ScrollViewProxy, animated: Bool) {
        self.ignoresScrollUntil = .now.addingTimeInterval(0.3)
        if let question, self.answerHeight > self.maxViewportHeight {
            self.followsLatest = false
            withAnimation(animated ? BobeMotion.standard : nil) {
                proxy.scrollTo(question.id, anchor: .top)
            }
            return
        }
        self.followsLatest = true
        withAnimation(animated ? BobeMotion.follow : nil) {
            proxy.scrollTo(Self.bottomAnchor, anchor: .bottom)
        }
    }
}

struct MessageRow: View {
    let message: ChatMessage
    var onPresented: @MainActor (String) async -> Void = { _ in }

    @State private var visible = false
    @State private var appActive = NSApp.isActive
    @Environment(\.theme) private var theme

    private var isUser: Bool {
        self.message.sender == .user
    }

    var body: some View {
        Group {
            if self.isUser {
                self.userBubble
            } else {
                self.reply
            }
        }
        .onGeometryChange(for: Bool.self) { geometry in
            guard let viewport = geometry.bounds(of: .scrollView) else { return false }
            let row = geometry.frame(in: .local)
            let visible = row.intersection(viewport)
            return MessagePresentation.isPresented(
                visibleHeight: visible.isNull ? 0 : visible.height,
                rowHeight: row.height,
                viewportHeight: viewport.height
            )
        } action: { self.visible = $0 }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            self.appActive = true
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didResignActiveNotification)) { _ in
            self.appActive = false
        }
        .task(id: self.presentationKey) {
            if self.visible, self.appActive, !self.isUser, self.message.isComplete, !self.message.isStreaming {
                await self.onPresented(self.message.id)
            }
        }
    }

    private var presentationKey: String {
        "\(self.message.id):\(self.visible):\(self.appActive):\(self.message.isComplete):\(self.message.isStreaming)"
    }

    private var userBubble: some View {
        HStack(alignment: .bottom, spacing: 6) {
            Spacer(minLength: 44)
            if self.message.isPending {
                Image(systemName: "clock")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(self.theme.colors.textMuted)
                    .accessibilityLabel(L10n.tr("overlay.chat.pending_suffix"))
            }
            Text(self.message.content)
                .bobeTextStyle(.chatBody)
                .lineSpacing(2)
                .foregroundStyle(self.theme.colors.text)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(
                    UnevenRoundedRectangle(
                        topLeadingRadius: 17, bottomLeadingRadius: 17, bottomTrailingRadius: 6, topTrailingRadius: 17,
                        style: .continuous
                    )
                    .fill(self.theme.accentSoft)
                )
                .opacity(self.message.isPending ? 0.65 : 1)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(L10n.tr("overlay.chat.sender.you")): \(self.message.content)")
    }

    private var reply: some View {
        VStack(alignment: .leading, spacing: 5) {
            if self.message.isStreaming {
                StreamingText(content: self.message.content)
            } else {
                CompletedReplyText(markdown: self.message.content)
            }
            if !self.message.isComplete, !self.message.isStreaming {
                Label(L10n.tr("overlay.chat.interrupted"), systemImage: "exclamationmark.circle")
                    .bobeTextStyle(.chatPending)
                    .foregroundStyle(self.theme.colors.error)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(
            "\(L10n.tr("overlay.chat.sender.bobe")): \(CompletedReplyText.plainText(fromMarkdown: self.message.content))"
        )
    }
}

enum MessagePresentation {
    static func isPresented(visibleHeight: CGFloat, rowHeight: CGFloat, viewportHeight: CGFloat) -> Bool {
        let required = min(rowHeight, viewportHeight) / 2
        return required > 0 && visibleHeight >= required
    }
}

struct CompletedReplyText: View {
    let markdown: String
    @Environment(\.theme) private var theme

    var body: some View {
        StructuredText(markdown: self.markdown)
            .textual.inlineStyle(ReplyMarkdownTheme.inlineStyle(for: self.theme))
            .textual.codeBlockStyle(ReplyCodeBlockStyle(background: self.theme.well))
            .textual.highlighterTheme(ReplyMarkdownTheme.highlighterTheme(for: self.theme))
            .textual.structuredTextStyle(.gitHub)
            .textual.textSelection(.enabled)
            .foregroundStyle(self.theme.colors.text)
            .environment(\.colorScheme, self.theme.isDark ? .dark : .light)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(Self.plainText(fromMarkdown: self.markdown))
    }

    nonisolated static func plainText(fromMarkdown markdown: String) -> String {
        let options = AttributedString.MarkdownParsingOptions(
            interpretedSyntax: .full,
            failurePolicy: .returnPartiallyParsedIfPossible
        )
        guard let attributed = try? AttributedString(markdown: markdown, options: options) else {
            return markdown
        }
        var blocks: [String] = []
        var block = ""
        var intent: PresentationIntent?
        for run in attributed.runs {
            if run.presentationIntent != intent, !block.isEmpty {
                blocks.append(block)
                block = ""
            }
            intent = run.presentationIntent
            block += String(attributed[run.range].characters)
        }
        blocks.append(block)
        return blocks
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
            .joined(separator: "\n")
    }
}

private enum ReplyMarkdownTheme {
    static func inlineStyle(for theme: ThemeConfig) -> InlineStyle {
        InlineStyle.gitHub.code(
            .monospaced,
            .fontScale(0.85),
            .foregroundColor(theme.colors.text),
            .backgroundColor(theme.well)
        )
    }

    static func highlighterTheme(for theme: ThemeConfig) -> StructuredText.HighlighterTheme {
        let colors = theme.colors
        let keyword = AnyTextProperty(.foregroundColor(colors.primary), .fontWeight(.semibold))
        let string = AnyTextProperty(.foregroundColor(colors.secondary))
        let symbol = AnyTextProperty(.foregroundColor(colors.tertiary))
        let comment = AnyTextProperty(.foregroundColor(colors.textMuted))
        return StructuredText.HighlighterTheme(
            foregroundColor: DynamicColor(colors.text),
            backgroundColor: DynamicColor(theme.well),
            tokenProperties: [
                .keyword: keyword,
                .literal: keyword,
                .boolean: keyword,
                .string: string,
                .char: string,
                .regex: string,
                .url: string,
                .attributeValue: string,
                .number: symbol,
                .constant: symbol,
                .className: symbol,
                .builtin: symbol,
                .function: symbol,
                .functionName: symbol,
                .attribute: symbol,
                .attributeName: symbol,
                .tag: symbol,
                .preprocessor: symbol,
                .directive: symbol,
                .comment: comment,
                .blockComment: comment,
                .docComment: comment,
                .inserted: AnyTextProperty(.foregroundColor(colors.success)),
                .deleted: AnyTextProperty(.foregroundColor(colors.error)),
            ]
        )
    }
}

private struct ReplyCodeBlockStyle: StructuredText.CodeBlockStyle {
    let background: Color

    func makeBody(configuration: Configuration) -> some View {
        Overflow {
            configuration.label
                .textual.lineSpacing(.fontScaled(0.225))
                .textual.fontScale(0.85)
                .fixedSize(horizontal: false, vertical: true)
                .monospaced()
                .padding(16)
        }
        .background(self.background)
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .textual.blockSpacing(.init(top: 0, bottom: 16))
    }
}

struct StreamingText: View {
    let content: String
    @Environment(\.theme) private var theme

    var body: some View {
        let caret = Text(Image(systemName: "circle.fill"))
            .font(.system(size: 7))
            .foregroundStyle(self.theme.colors.primary)
        return Text("\(Text(verbatim: self.content))  \(caret)")
            .bobeTextStyle(.chatBody)
            .lineSpacing(3)
            .foregroundStyle(self.theme.colors.text)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

struct LiveActivityLine: View {
    let activity: String?
    let tools: [ToolExecution]
    @Environment(\.theme) private var theme

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            if !self.tools.isEmpty {
                FlowLayout(spacing: 6) {
                    ForEach(self.tools) { tool in
                        ToolChip(tool: tool)
                            .transition(.bobePop(anchor: .leading))
                    }
                }
                .animation(BobeMotion.snappy, value: self.tools.map(\.id))
            }
            if let activity {
                HStack(spacing: 7) {
                    BobeBreathingDot()
                    BobeShimmerText(text: activity)
                        .id(activity)
                        .transition(.bobeRise)
                }
                .frame(height: 20)
                .animation(BobeMotion.snappy, value: activity)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

struct ToolChip: View {
    let tool: ToolExecution
    @Environment(\.theme) private var theme

    var body: some View {
        HStack(spacing: 5) {
            self.mark
                .frame(width: 12, height: 12)
            Text(Self.displayName(self.tool.toolName))
                .font(.system(size: 11.5, weight: .medium))
                .foregroundStyle(self.theme.colors.textMuted)
                .lineLimit(1)
        }
        .padding(.horizontal, 9)
        .frame(height: 24)
        .bobeWell(in: .bobePill)
    }

    @ViewBuilder
    private var mark: some View {
        switch self.tool.status {
        case .running:
            BobeSpinner(size: 10, lineWidth: 1.6, color: self.theme.glow[1])
        case .success:
            Image(systemName: "checkmark")
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(self.theme.colors.success)
                .transition(.bobePop())
        case .error:
            Image(systemName: "xmark")
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(self.theme.colors.error)
                .transition(.bobePop())
        }
    }

    static func displayName(_ raw: String) -> String {
        raw.replacingOccurrences(of: "_", with: " ").replacingOccurrences(of: "-", with: " ")
    }
}
