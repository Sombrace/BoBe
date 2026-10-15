import AppKit
import SwiftUI

enum OverlaySatelliteFocus: Hashable {
    case chat
    case microphone
    case menu
}

enum OverlayPresentation: Equatable {
    case collapsed
    case ambient
    case ask
    case conversation

    var columnWidth: CGFloat {
        switch self {
        case .collapsed: 0
        case .ambient: WindowSizes.columnAmbient
        case .ask: WindowSizes.columnAsk
        case .conversation: WindowSizes.columnConversation
        }
    }

    var columnHeight: CGFloat {
        let window: CGFloat = switch self {
        case .collapsed: WindowSizes.heightCollapsed
        case .ambient: WindowSizes.heightAmbient
        case .ask: WindowSizes.heightAsk
        case .conversation: WindowSizes.heightConversation
        }
        return window - WindowSizes.shadowInset - WindowSizes.columnBaseline - WindowSizes.clusterBottom
    }
}

@MainActor
@Observable
final class OverlayModel {
    var isChatVisible = false
    var isRecentConversationVisible = false
    var draftMessage = ""
    var lastMessageActivity: Date = .now
    var latestReplyAt: Date = .distantPast
    var measuredColumnSize: CGSize = .zero
    @ObservationIgnored var inactivityTimer: Task<Void, Never>?
    @ObservationIgnored var resizeTask: Task<Void, Never>?
    var composerFeedback: String?
    var celebrationToken = 0
    var isPointerOverChat = false
    var voiceCoachmarkVisible = false
    var floatingBubbleMessageId: String?
    var floatingBubbleDismissalStyle: AmbientBubbleDismissalStyle = .fade
    var floatingBubbleRecedeProgress: Double = 0
    var floatingBubbleTransitionGeneration = 0
    var floatingBubbleInteractionActive = false
    @ObservationIgnored var floatingBubbleAutoDismissTask: Task<Void, Never>?
    var floatingBubbleWasEngaged = false
    var reachoutKinds: [String: ReachoutKind] = [:]
    var reachoutInbox = ReachoutInbox()
    var avatarAreaHovered = false
    var satellitesShown = false
    @ObservationIgnored var satelliteHideTask: Task<Void, Never>?
}

struct OverlayView: View {
    enum Surface {
        case avatar
        case column
    }

    let surface: Surface
    let model: OverlayModel
    @State var store: BobeStore
    @State var themeStore: ThemeStore
    @FocusState var focusedSatellite: OverlaySatelliteFocus?

    init(surface: Surface, model: OverlayModel, store: BobeStore, themeStore: ThemeStore = .shared) {
        self.surface = surface
        self.model = model
        self._store = State(initialValue: store)
        self._themeStore = State(initialValue: themeStore)
    }

    var body: some View {
        switch self.surface {
        case .avatar: self.lifecycleObservedOverlay
        case .column: self.staged(self.columnRoot)
        }
    }

    private func staged(_ content: some View) -> some View {
        content
            .environment(\.theme, self.themeStore.currentTheme)
            .environment(\.overlayPlacement, self.placement)
    }

    private var baseOverlay: some View {
        self.staged(self.avatarRoot)
    }

    var stage: OverlayStage {
        OverlayWindowManager.shared.stage
    }

    var placement: OverlayPlacement {
        self.stage.placement
    }

    var isChatVisible: Bool {
        get { self.model.isChatVisible }
        nonmutating set { self.model.isChatVisible = newValue }
    }

    var isRecentConversationVisible: Bool {
        get { self.model.isRecentConversationVisible }
        nonmutating set { self.model.isRecentConversationVisible = newValue }
    }

    var draftMessage: String {
        get { self.model.draftMessage }
        nonmutating set { self.model.draftMessage = newValue }
    }

    var lastMessageActivity: Date {
        get { self.model.lastMessageActivity }
        nonmutating set { self.model.lastMessageActivity = newValue }
    }

    var latestReplyAt: Date {
        get { self.model.latestReplyAt }
        nonmutating set { self.model.latestReplyAt = newValue }
    }

    var measuredColumnSize: CGSize {
        get { self.model.measuredColumnSize }
        nonmutating set { self.model.measuredColumnSize = newValue }
    }

    var inactivityTimer: Task<Void, Never>? {
        get { self.model.inactivityTimer }
        nonmutating set { self.model.inactivityTimer = newValue }
    }

    var resizeTask: Task<Void, Never>? {
        get { self.model.resizeTask }
        nonmutating set { self.model.resizeTask = newValue }
    }

    var composerFeedback: String? {
        get { self.model.composerFeedback }
        nonmutating set { self.model.composerFeedback = newValue }
    }

    var celebrationToken: Int {
        get { self.model.celebrationToken }
        nonmutating set { self.model.celebrationToken = newValue }
    }

    var isPointerOverChat: Bool {
        get { self.model.isPointerOverChat }
        nonmutating set { self.model.isPointerOverChat = newValue }
    }

    var voiceCoachmarkVisible: Bool {
        get { self.model.voiceCoachmarkVisible }
        nonmutating set { self.model.voiceCoachmarkVisible = newValue }
    }

    var floatingBubbleMessageId: String? {
        get { self.model.floatingBubbleMessageId }
        nonmutating set { self.model.floatingBubbleMessageId = newValue }
    }

    var floatingBubbleDismissalStyle: AmbientBubbleDismissalStyle {
        get { self.model.floatingBubbleDismissalStyle }
        nonmutating set { self.model.floatingBubbleDismissalStyle = newValue }
    }

    var floatingBubbleRecedeProgress: Double {
        get { self.model.floatingBubbleRecedeProgress }
        nonmutating set { self.model.floatingBubbleRecedeProgress = newValue }
    }

    var floatingBubbleTransitionGeneration: Int {
        get { self.model.floatingBubbleTransitionGeneration }
        nonmutating set { self.model.floatingBubbleTransitionGeneration = newValue }
    }

    var floatingBubbleInteractionActive: Bool {
        get { self.model.floatingBubbleInteractionActive }
        nonmutating set { self.model.floatingBubbleInteractionActive = newValue }
    }

    var floatingBubbleAutoDismissTask: Task<Void, Never>? {
        get { self.model.floatingBubbleAutoDismissTask }
        nonmutating set { self.model.floatingBubbleAutoDismissTask = newValue }
    }

    var floatingBubbleWasEngaged: Bool {
        get { self.model.floatingBubbleWasEngaged }
        nonmutating set { self.model.floatingBubbleWasEngaged = newValue }
    }

    var reachoutKinds: [String: ReachoutKind] {
        get { self.model.reachoutKinds }
        nonmutating set { self.model.reachoutKinds = newValue }
    }

    var reachoutInbox: ReachoutInbox {
        get { self.model.reachoutInbox }
        nonmutating set { self.model.reachoutInbox = newValue }
    }

    var avatarAreaHovered: Bool {
        get { self.model.avatarAreaHovered }
        nonmutating set { self.model.avatarAreaHovered = newValue }
    }

    var satellitesShown: Bool {
        get { self.model.satellitesShown }
        nonmutating set { self.model.satellitesShown = newValue }
    }

    var satelliteHideTask: Task<Void, Never>? {
        get { self.model.satelliteHideTask }
        nonmutating set { self.model.satelliteHideTask = newValue }
    }

    private var conversationObservedOverlay: some View {
        self.baseOverlay
            .task {
                if SetupWindowManager.shared.consumeOpenChatAfterOnboarding() {
                    await Task.yield()
                    self.openChatManually()
                }
            }
            .onChange(of: self.store.messages.count) { oldCount, newCount in
                self.handleMessagesChange(oldCount: oldCount, newCount: newCount)
            }
            .onChange(of: self.store.latestLiveBobeMessageId) { _, messageId in
                self.handleLiveBobeMessageChange(messageId)
            }
            .onChange(of: self.store.messages.last?.isStreaming) { wasStreaming, isStreaming in
                if wasStreaming == true, isStreaming == false,
                   self.store.messages.last?.sender == .bobe,
                   self.store.messages.last?.isComplete == true {
                    self.celebrationToken &+= 1
                    self.latestReplyAt = .now
                }
            }
            .onChange(of: self.store.failedSendRecoveries.count) { _, recoveryCount in
                if recoveryCount > 0, !self.isChatVisible {
                    self.isRecentConversationVisible = false
                    self.setChatVisible(true)
                }
            }
            .onChange(of: self.store.composerBlockReason) { _, newReason in
                if let newReason, self.composerFeedback != nil {
                    self.composerFeedback = self.waitingMessage(for: newReason)
                } else if newReason == nil {
                    self.composerFeedback = nil
                }
            }
            .onChange(of: self.draftMessage) { _, newValue in
                self.lastMessageActivity = .now
                if newValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    self.composerFeedback = nil
                }
            }
            .onChange(of: self.presentation) { _, _ in
                self.syncWindow()
            }
            .onChange(of: self.placement) { _, _ in
                self.scheduleWindowSettle()
            }
            .onChange(of: self.stage.columnHeightLimit) { _, _ in
                self.syncWindow()
            }
            .onChange(of: self.stage.escapeCount) { _, _ in
                self.handleEscape()
            }
            .onChange(of: self.stage.clickAwayCount) { _, _ in
                self.handleClickAway()
            }
            .onChange(of: self.isChatVisible) { _, visible in
                OverlayWindowManager.shared.setChatOpen(visible)
            }
            .onChange(of: self.stage.pointerOverAvatar) { _, hovered in
                self.avatarAreaHovered = hovered
            }
            .onChange(of: self.stage.pointerOverAvatar || self.stage.pointerOverContent) { _, hovering in
                self.isPointerOverChat = hovering
                if hovering {
                    self.lastMessageActivity = .now
                }
            }
            .onChange(of: self.satellitesShown || self.isChatVisible, initial: true) { _, exposed in
                OverlayWindowManager.shared.setSatellitesExposed(exposed)
            }
    }

    private var activityObservedOverlay: some View {
        self.conversationObservedOverlay
            .onChange(of: self.isTtsAudible) { _, audible in
                if audible {
                    self.cancelFloatingBubbleAutoDismiss()
                } else {
                    self.scheduleFloatingBubbleAutoDismissIfReady()
                }
            }
            .onChange(of: VoicePipeline.shared.state) { _, state in
                if state == .listening || state == .capturing,
                   !VoiceCoachmarkFlag.hasSeen,
                   !BobeMotion.isReduced {
                    VoiceCoachmarkFlag.markSeen()
                    withAnimation(BobeMotion.standard) {
                        self.voiceCoachmarkVisible = true
                    }
                    Task { @MainActor in
                        try? await Task.sleep(for: .seconds(6))
                        withAnimation(BobeMotion.standard) {
                            self.voiceCoachmarkVisible = false
                        }
                    }
                }
                if state == .thinking || state == .speaking {
                    self.cancelFloatingBubbleAutoDismiss()
                } else {
                    self.scheduleFloatingBubbleAutoDismissIfReady()
                }
            }
    }

    private var bubbleObservedOverlay: some View {
        self.activityObservedOverlay
            .onChange(of: self.floatingBubbleMessage?.id) { _, newId in
                if newId == nil {
                    self.cancelFloatingBubbleAutoDismiss()
                } else {
                    self.scheduleFloatingBubbleAutoDismissIfReady()
                }
            }
            .onChange(of: self.floatingBubbleMessage?.isStreaming) { _, streaming in
                if streaming == false {
                    self.scheduleFloatingBubbleAutoDismissIfReady()
                }
            }
            .onChange(of: self.avatarAreaHovered) { _, hovered in
                self.handleAvatarHoverChange(hovered)
                self.handleFloatingBubbleInteractionChange(
                    hovered || self.floatingBubbleInteractionActive
                )
            }
            .onChange(of: self.floatingBubbleInteractionActive) { _, active in
                self.handleFloatingBubbleInteractionChange(active || self.avatarAreaHovered)
            }
            .onChange(of: self.isRecentConversationVisible) { _, visible in
                if visible {
                    self.cancelFloatingBubbleAutoDismiss()
                } else {
                    self.scheduleFloatingBubbleAutoDismissIfReady()
                }
            }
    }

    private var lifecycleObservedOverlay: some View {
        self.bubbleObservedOverlay
            .onAppear {
                self.syncWindow()
                self.startInactivityTimer()
            }
            .onDisappear {
                self.inactivityTimer?.cancel()
                self.resizeTask?.cancel()
                self.satelliteHideTask?.cancel()
                self.cancelFloatingBubbleAutoDismiss()
                OverlayWindowManager.shared.setChatOpen(false)
            }
    }

    var ambientConversationMessages: [ChatMessage] {
        self.store.messages.filter(\.belongsInConversationTrace)
    }

    var currentExchange: [ChatMessage] {
        let all = self.ambientConversationMessages
        guard let lastUser = all.lastIndex(where: { $0.sender == .user }) else {
            return Array(all.suffix(1))
        }
        let start = lastUser > 0 && all[lastUser - 1].sender == .bobe ? lastUser - 1 : lastUser
        return Array(all[start...])
    }

    var earlierMessages: [ChatMessage] {
        let all = self.ambientConversationMessages
        return Array(all.prefix(all.count - self.currentExchange.count))
    }

    var liveActivity: String? {
        if let tool = self.store.runningTools.first {
            return L10n.tr("overlay.status.using_tool_format", ToolChip.displayName(tool.toolName))
        }
        guard self.store.isThinking, self.store.messages.last?.isStreaming != true else { return nil }
        return L10n.tr("overlay.status.thinking")
    }

    var floatingBubbleMessage: ChatMessage? {
        guard !self.isChatVisible,
              !self.store.proactiveSilenced,
              let id = self.floatingBubbleMessageId
        else { return nil }
        return self.store.messages.first(where: { $0.id == id })
    }

    var overlaySectionTransition: AnyTransition {
        .bobeRise
    }

    var avatarMood: BobeMood {
        if self.store.isInitialConnectionPending {
            return .waking
        }
        if self.store.context.daemonError || self.store.isReconnecting {
            return .worried
        }
        if !self.store.isConnected, !self.store.isBackendFatal {
            return .idle
        }
        let voice = VoicePipeline.shared.state
        if voice == .listening || voice == .capturing {
            return .listening
        }
        if self.isTtsAudible {
            return .speaking
        }
        switch self.store.stateType {
        case .loading: return .waking
        case .error: return .worried
        case .thinking: return self.store.runningTools.isEmpty ? .thinking : .working
        case .speaking: return .speaking
        case .wantsToSpeak: return self.wantsAttention ? .eager : .idle
        case .shuttingDown: return .asleep
        case .idle:
            if voice == .thinking { return .thinking }
            return self.store.isReadingScreen ? .watching : .idle
        }
    }

    var wantsAttention: Bool {
        guard !self.isChatVisible, self.floatingBubbleMessage == nil, !self.store.proactiveSilenced else {
            return false
        }
        return self.store.stateType == .wantsToSpeak || !self.reachoutInbox.isEmpty
    }

    var floatingBubbleKind: ReachoutKind {
        guard let message = self.floatingBubbleMessage else { return .answer }
        return self.reachoutKinds[message.id] ?? ReachoutKind(origin: message.responseOrigin, run: nil)
    }

    var waitingReachoutCount: Int {
        self.isChatVisible ? 0 : self.reachoutInbox.count
    }

    var isAvatarEngaged: Bool {
        self.avatarAreaHovered || self.isChatVisible || self.bubbleMode.isVisible
    }

    var bubbleMode: AvatarStatusBubble.Mode {
        if self.isChatVisible {
            return .hidden
        }
        let pipeline = VoicePipeline.shared
        if SettingsStore.shared.settings?.voiceShowPartialCaption ?? true, !pipeline.partialTranscript.isEmpty {
            return .userTranscribing(text: pipeline.partialTranscript)
        }
        if let msg = self.floatingBubbleMessage {
            return .bobeMessage(msg)
        }
        if self.store.isReconnecting {
            return .status(kind: .reconnecting)
        }
        if self.store.isInitialConnectionPending {
            return .status(kind: .starting)
        }
        if let tool = self.store.runningTools.first {
            return .status(kind: .toolRunning(name: tool.toolName))
        }
        if self.store.isThinking {
            return .status(kind: .thinking)
        }
        if self.store.proactiveSilenced {
            return .silenced
        }
        return .hidden
    }

    var isTtsAudible: Bool {
        let voice = VoicePipeline.shared
        return voice.state == .speaking || voice.isTtsAudible
    }

    var isVoiceResponseInProgress: Bool {
        let state = VoicePipeline.shared.state
        return state == .thinking || state == .speaking || state == .cancelling
    }

    var isVoiceStoppable: Bool {
        let state = VoicePipeline.shared.state
        return state == .thinking || state == .speaking
    }

    var hasAmbientNotice: Bool {
        self.store.softWarning != nil
            || self.store.screenUnavailableNotice
            || self.store.context.daemonError
            || self.store.errorMessage != nil
    }

    var presentation: OverlayPresentation {
        if self.isChatVisible {
            return self.isRecentConversationVisible ? .conversation : .ask
        }
        if self.bubbleMode.isVisible || self.hasAmbientNotice || self.voiceCoachmarkVisible {
            return .ambient
        }
        return .collapsed
    }

    var showMicSatellite: Bool {
        self.satellitesShown || self.isChatVisible || self.avatarMood == .listening
    }

    var showChatToggleSatellite: Bool {
        self.satellitesShown || self.isChatVisible
    }

    var columnHeightLimit: CGFloat {
        max(OverlayCanvas.minimumColumnHeight, self.stage.columnHeightLimit)
    }

    var chatViewportMaxHeight: CGFloat {
        let ceiling = min(self.presentation.columnHeight, self.columnHeightLimit)
        return max(
            WindowSizes.heightChatViewportMin,
            min(WindowSizes.heightChatViewportMax, ceiling - WindowSizes.heightThreadChrome)
        )
    }
}

extension AvatarStatusBubble.Mode {
    var isVisible: Bool {
        if case .hidden = self { return false }
        return true
    }
}
