import Foundation
import SwiftUI

extension OverlayView {
    @discardableResult
    func handleSendMessage(_ content: String) -> Bool {
        self.lastMessageActivity = .now

        if let blockReason = self.store.composerBlockReason {
            self.composerFeedback = self.waitingMessage(for: blockReason)
            return false
        }

        self.composerFeedback = nil
        Task {
            await self.store.sendMessage(content)
        }
        self.dismissFloatingBubble()
        return true
    }

    func handleMessagesChange(oldCount _: Int, newCount: Int) {
        if newCount == 0 {
            self.composerFeedback = nil
            self.dismissFloatingBubble()
            self.reachoutKinds = [:]
            withAnimation(BobeMotion.soft) {
                self.isRecentConversationVisible = false
            }
        }
        self.reachoutInbox.prune(keeping: Set(self.store.messages.map(\.id)))
        self.lastMessageActivity = .now
    }

    func handleLiveBobeMessageChange(_ messageId: String?) {
        guard let messageId,
              let message = self.store.messages.first(where: { $0.id == messageId })
        else { return }
        self.latestReplyAt = .now
        self.classifyReachout(message)
        if message.responseOrigin == .proactive, self.store.proactiveSilenced, !self.isChatVisible {
            withAnimation(BobeMotion.standard) {
                self.reachoutInbox.hold(message.id)
            }
            return
        }
        self.surfaceFloatingBubble(for: message)
    }

    func classifyReachout(_ message: ChatMessage) {
        guard self.reachoutKinds[message.id] == nil else { return }
        let origin = message.responseOrigin
        self.reachoutKinds[message.id] = ReachoutKind(origin: origin, run: nil)
        guard origin == .proactive else { return }
        let selection = EndpointStore.shared.selection
        Task { @MainActor in
            guard let runs = try? await DaemonClient(endpoint: selection.endpoint).agentRuns(),
                  EndpointStore.shared.selection == selection,
                  let carrier = ReachoutKind.carrier(ofMessage: message.id, in: runs)
            else { return }
            withAnimation(BobeMotion.standard) {
                self.reachoutKinds[message.id] = ReachoutKind(origin: origin, run: carrier)
            }
        }
    }

    func settleFloatingBubble() {
        if let id = self.floatingBubbleMessageId {
            self.reachoutInbox.settle(id)
        }
        self.dismissFloatingBubble()
    }

    func tellMeMore() {
        self.openCurrentAnswer()
        self.handleSendMessage(L10n.tr("overlay.reachout.more.message"))
    }

    func peekWaitingReachout() {
        guard !self.isChatVisible,
              !self.store.proactiveSilenced,
              self.floatingBubbleMessageId == nil,
              let id = self.reachoutInbox.latest,
              let message = self.store.messages.first(where: { $0.id == id })
        else { return }
        withAnimation(BobeMotion.standard) {
            self.reachoutInbox.settle(id)
        }
        self.surfaceFloatingBubble(for: message)
        self.floatingBubbleWasEngaged = true
    }

    func handleAvatarTap() {
        if !self.isChatVisible, self.floatingBubbleMessage != nil {
            self.openCurrentAnswer()
        } else {
            self.toggleChatManually()
        }
    }

    func handleAvatarHoverChange(_ hovered: Bool) {
        self.satelliteHideTask?.cancel()
        if hovered {
            self.satellitesShown = true
            self.peekWaitingReachout()
            return
        }
        self.satelliteHideTask = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(AvatarMetrics.satelliteLingerMilliseconds))
            guard !Task.isCancelled else { return }
            self.satellitesShown = false
        }
    }

    func toggleChatManually() {
        if self.isChatVisible {
            self.closeChat(userInitiated: true)
        } else {
            self.openChatManually()
        }
    }

    func openChatManually() {
        self.isRecentConversationVisible = false
        self.setChatVisible(true)
        OverlayWindowManager.shared.focusForTyping()
    }

    func openCurrentAnswer() {
        self.cancelFloatingBubbleAutoDismiss()
        self.setChatVisible(true)
        self.dismissFloatingBubble()
        OverlayWindowManager.shared.focusForTyping()
    }

    func closeChat(userInitiated _: Bool) {
        self.setChatVisible(false)
        self.dismissFloatingBubble()
    }

    func toggleProactiveSilence() {
        self.store.toggleProactiveSilence()
        if self.store.proactiveSilenced {
            self.dismissFloatingBubble()
        }
    }

    func surfaceFloatingBubble(for message: ChatMessage) {
        guard message.hasVisibleContent,
              !self.store.proactiveSilenced,
              !self.isChatVisible
        else { return }
        if let current = self.floatingBubbleMessage,
           current.id != message.id,
           !self.floatingBubbleWasEngaged,
           self.floatingBubbleKind.waitsWhenMissed {
            self.reachoutInbox.hold(current.id)
        }
        self.preflightWindow(for: .ambient)
        self.floatingBubbleWasEngaged = false
        self.floatingBubbleDismissalStyle = .fade
        self.floatingBubbleRecedeProgress = 0
        self.floatingBubbleTransitionGeneration &+= 1
        withAnimation(BobeMotion.standard) {
            self.floatingBubbleMessageId = message.id
        }
        self.cancelFloatingBubbleAutoDismiss()
        self.scheduleFloatingBubbleAutoDismissIfReady()
    }

    func dismissFloatingBubble(style: AmbientBubbleDismissalStyle = .fade) {
        self.cancelFloatingBubbleAutoDismiss()
        self.floatingBubbleDismissalStyle = style
        guard self.floatingBubbleMessageId != nil else {
            return
        }

        self.floatingBubbleTransitionGeneration &+= 1
        let generation = self.floatingBubbleTransitionGeneration

        guard style == .stardust else {
            withAnimation(BobeMotion.standard) {
                self.floatingBubbleMessageId = nil
            }
            self.floatingBubbleRecedeProgress = 0
            return
        }

        let duration = BobeMotion.isReduced
            ? AmbientBubbleTransition.fadeDurationSeconds
            : AmbientBubbleTransition.stardustDurationSeconds
        let animation = BobeMotion.isReduced
            ? Animation.easeOut(duration: duration)
            : Animation.easeIn(duration: duration)

        withAnimation(animation, completionCriteria: .logicallyComplete) {
            self.floatingBubbleRecedeProgress = 1
        } completion: {
            guard generation == self.floatingBubbleTransitionGeneration else {
                return
            }
            var transaction = Transaction()
            transaction.disablesAnimations = true
            withTransaction(transaction) {
                self.floatingBubbleMessageId = nil
                self.floatingBubbleRecedeProgress = 0
            }
        }
    }

    func scheduleFloatingBubbleAutoDismissIfReady() {
        guard let message = self.floatingBubbleMessage,
              AmbientMessageTiming.shouldAutoDismiss(message),
              !message.isStreaming,
              !self.isTtsAudible,
              !self.isVoiceResponseInProgress,
              !self.avatarAreaHovered,
              !self.floatingBubbleInteractionActive
        else {
            return
        }
        self.scheduleFloatingBubbleAutoDismiss(
            after: AmbientMessageTiming.dwellDuration(for: message.content)
        )
    }

    func scheduleFloatingBubbleAutoDismiss(after delay: TimeInterval) {
        self.cancelFloatingBubbleAutoDismiss()
        self.floatingBubbleAutoDismissTask = Task { @MainActor in
            try? await Task.sleep(for: .seconds(delay))
            guard !Task.isCancelled else { return }
            self.floatingBubbleAutoDismissTask = nil
            if let message = self.floatingBubbleMessage,
               !self.floatingBubbleWasEngaged,
               self.floatingBubbleKind.waitsWhenMissed {
                withAnimation(BobeMotion.standard) {
                    self.reachoutInbox.hold(message.id)
                }
            }
            self.dismissFloatingBubble(style: .stardust)
        }
    }

    func cancelFloatingBubbleAutoDismiss() {
        self.floatingBubbleAutoDismissTask?.cancel()
        self.floatingBubbleAutoDismissTask = nil
    }

    func handleFloatingBubbleInteractionChange(_ active: Bool) {
        if active {
            self.floatingBubbleWasEngaged = true
            self.cancelFloatingBubbleAutoDismiss()
            guard self.floatingBubbleRecedeProgress > 0 else {
                return
            }
            self.floatingBubbleTransitionGeneration &+= 1
            withAnimation(
                BobeMotion.isReduced
                    ? .easeOut(duration: 0.1)
                    : .easeOut(duration: 0.14)
            ) {
                self.floatingBubbleRecedeProgress = 0
            }
        } else {
            self.scheduleFloatingBubbleAutoDismissIfReady()
        }
    }

    func setChatVisible(_ visible: Bool) {
        guard self.isChatVisible != visible else { return }
        if visible {
            self.preflightWindow(for: self.isRecentConversationVisible ? .conversation : .ask)
            self.reachoutInbox.settleAll()
        }
        withAnimation(BobeMotion.standard) {
            if !visible {
                self.isRecentConversationVisible = false
            }
            self.isChatVisible = visible
        }
        self.lastMessageActivity = .now
    }

    func toggleRecentConversation() {
        if !self.isRecentConversationVisible {
            self.preflightWindow(for: .conversation)
        }
        withAnimation(BobeMotion.soft) {
            self.isRecentConversationVisible.toggle()
        }
        self.lastMessageActivity = .now
    }

    func preflightWindow(for presentation: OverlayPresentation) {
        let column = CGSize(
            width: presentation.columnWidth,
            height: min(presentation.columnHeight, self.columnHeightLimit)
        )
        OverlayWindowManager.shared.reveal(OverlayCanvas.columnViewport(size: column, placement: self.placement))
        self.scheduleWindowSettle()
    }

    func syncWindow() {
        let target = self.settledViewport()
        let manager = OverlayWindowManager.shared
        if let target {
            manager.reveal(target)
        }
        if manager.requested != target {
            self.scheduleWindowSettle()
        }
    }

    func scheduleWindowSettle() {
        self.resizeTask?.cancel()
        self.resizeTask = Task { @MainActor in
            try? await Task.sleep(for: .milliseconds(WindowSizes.settleMilliseconds))
            guard !Task.isCancelled else { return }
            OverlayWindowManager.shared.settle(self.settledViewport())
        }
    }

    func settledViewport() -> CGRect? {
        let presentation = self.presentation
        guard presentation != .collapsed else { return nil }
        let column = CGSize(
            width: presentation.columnWidth,
            height: min(self.columnHeightLimit, self.measuredColumnSize.height)
        )
        return OverlayCanvas.columnViewport(size: column, placement: self.placement)
    }

    func handleEscape() {
        if self.isChatVisible {
            self.closeChat(userInitiated: true)
        } else if self.floatingBubbleMessageId != nil {
            self.settleFloatingBubble()
        }
    }

    func handleClickAway() {
        guard self.isChatVisible,
              self.draftMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else { return }
        self.recedeChat()
    }

    func recedeChat() {
        let latest = self.ambientConversationMessages.last
        let isFresh = Date.now.timeIntervalSince(self.latestReplyAt) < AmbientMessageTiming.recedeFreshSeconds
        self.setChatVisible(false)
        guard let latest, latest.sender == .bobe, latest.hasVisibleContent,
              latest.isStreaming || isFresh
        else {
            self.dismissFloatingBubble()
            return
        }
        self.surfaceFloatingBubble(for: latest)
        self.floatingBubbleWasEngaged = true
    }

    func startInactivityTimer() {
        self.inactivityTimer?.cancel()
        self.inactivityTimer = Task { @MainActor in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(InactivityTiming.checkIntervalSeconds))
                guard !Task.isCancelled else { return }
                let elapsed = Date.now.timeIntervalSince(self.lastMessageActivity)
                let isResting = self.isChatVisible
                    && self.store.stateType == .idle
                    && !self.isPointerOverChat
                    && self.draftMessage.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                guard isResting else { continue }
                if elapsed > InactivityTiming.timeoutSeconds {
                    self.closeChat(userInitiated: false)
                } else if elapsed > InactivityTiming.settleSeconds, self.isRecentConversationVisible {
                    withAnimation(BobeMotion.soft) {
                        self.isRecentConversationVisible = false
                    }
                }
            }
        }
    }

    func waitingMessage(for reason: MessageComposerBlockReason) -> String {
        switch reason {
        case .starting:
            L10n.tr("overlay.input.waiting_for_starting")
        case .reconnecting:
            L10n.tr("overlay.input.waiting_for_reconnecting")
        case .thinking:
            L10n.tr("overlay.input.waiting_for_thinking")
        }
    }
}

enum AmbientMessageTiming {
    static let minimumDwellSeconds: TimeInterval = 8
    static let recedeFreshSeconds: TimeInterval = 120
    static let maximumAutoDismissWords = 60

    static func dwellDuration(for content: String) -> TimeInterval {
        let wordCount = content.split(whereSeparator: \.isWhitespace).count
        return max(self.minimumDwellSeconds, 2 + Double(wordCount) / 3.3)
    }

    static func shouldAutoDismiss(_ message: ChatMessage) -> Bool {
        guard message.responseOrigin == .proactive, message.isComplete else {
            return false
        }
        return message.content.split(whereSeparator: \.isWhitespace).count
            <= self.maximumAutoDismissWords
    }
}
