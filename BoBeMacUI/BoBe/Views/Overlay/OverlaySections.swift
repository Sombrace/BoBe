import SwiftUI

extension OverlayView {
    var avatarRoot: some View {
        self.avatarCluster
            .frame(
                width: OverlayCanvas.clusterSize.width,
                height: OverlayCanvas.clusterSize.height,
                alignment: self.placement.satellitesBelow ? .bottomLeading : .topLeading
            )
    }

    var columnRoot: some View {
        ZStack(alignment: self.placement.alignment) {
            if self.presentation != .collapsed {
                self.conversationColumn
                    .frame(width: self.presentation.columnWidth, alignment: self.placement.alignment)
                    .onGeometryChange(for: CGSize.self) { geometry in
                        CGSize(width: ceil(geometry.size.width), height: ceil(geometry.size.height))
                    } action: { newSize in
                        guard newSize.height > 0,
                              abs(newSize.height - self.measuredColumnSize.height) > 0.5
                              || abs(newSize.width - self.measuredColumnSize.width) > 0.5
                        else { return }
                        self.measuredColumnSize = newSize
                        self.syncWindow()
                    }
                    .transition(self.columnTransition)
            }
        }
        .padding(WindowSizes.shadowInset)
        .frame(
            width: OverlayCanvas.columnCanvas.width,
            height: OverlayCanvas.columnCanvas.height,
            alignment: self.placement.alignment
        )
        .coordinateSpace(.named(OverlayCanvas.space))
        .animation(BobeMotion.soft, value: self.presentation)
        .onPreferenceChange(OverlayHitRegionKey.self) { regions in
            MainActor.assumeIsolated {
                OverlayWindowManager.shared.setHitRegions(regions)
            }
        }
    }

    private var columnTransition: AnyTransition {
        .bobePop(anchor: self.placement.corner, scale: 0.9)
    }

    private var capsuleTransition: AnyTransition {
        .bobePop(anchor: self.placement.nearEdge, scale: 0.4)
    }

    private var showsThread: Bool {
        !self.ambientConversationMessages.isEmpty || self.liveActivity != nil || !self.store.toolExecutions.isEmpty
    }

    var conversationColumn: some View {
        AnchoredStack(
            hugsTrailing: self.placement.hugsTrailing,
            reversed: self.placement.growth == .down,
            spacing: 8
        ) {
            if self.isChatVisible {
                if self.showsThread {
                    ConversationSheet(
                        earlier: self.earlierMessages,
                        current: self.currentExchange,
                        isExpanded: self.isRecentConversationVisible,
                        onToggleExpanded: self.toggleRecentConversation,
                        activity: self.liveActivity,
                        tools: self.store.toolExecutions,
                        maxViewportHeight: self.chatViewportMaxHeight,
                        onPresented: self.store.markResponsePresented
                    )
                    .overlayHitRegion()
                    .transition(self.overlaySectionTransition)
                }
                OwnerQuestionsView(selection: self.store.selection)
                    .overlayHitRegion()
                self.conversationEndingSection
                self.recoverySection
            } else if self.voiceCoachmarkVisible {
                VoiceCoachmark(isVisible: Bindable(self.model).voiceCoachmarkVisible)
                    .overlayHitRegion()
            } else {
                AvatarStatusBubble(
                    mode: self.bubbleMode,
                    onOpenAnswer: { self.openCurrentAnswer() },
                    onToggleSilence: { self.toggleProactiveSilence() },
                    onDismiss: { self.settleFloatingBubble() },
                    isSilenced: self.store.proactiveSilenced,
                    isTtsAudible: self.isTtsAudible,
                    dismissalStyle: self.floatingBubbleDismissalStyle,
                    recedeProgress: self.floatingBubbleRecedeProgress,
                    onInteractionChanged: { self.floatingBubbleInteractionActive = $0 },
                    onStop: self.isVoiceStoppable ? { VoicePipeline.shared.interrupt() } : nil,
                    reachout: self.floatingBubbleKind,
                    onReply: { self.openCurrentAnswer() },
                    onTellMore: { self.tellMeMore() },
                    onSettle: { self.settleFloatingBubble() }
                )
                .padding(self.placement.nearVerticalEdge, self.bubbleMode.isPill ? 9 : 0)
                .transition(self.overlaySectionTransition)
            }
            self.softWarningSection
            self.screenUnavailableSection
            self.errorBannerSection
            if self.isChatVisible {
                AskCapsule(
                    text: Bindable(self.model).draftMessage,
                    onSend: self.handleSendMessage,
                    onClose: { self.closeChat(userInitiated: true) },
                    selection: self.store.selection,
                    feedbackMessage: self.composerFeedback,
                    isWorking: self.store.composerBlockReason == .thinking,
                    canStopVoice: self.isVoiceStoppable,
                    onStopVoice: { VoicePipeline.shared.interrupt() }
                )
                .overlayHitRegion()
                .transition(self.capsuleTransition)
            }
        }
        .animation(BobeMotion.standard, value: self.showsThread)
        .animation(BobeMotion.standard, value: self.hasAmbientNotice)
        .animation(BobeMotion.standard, value: self.store.failedSendRecoveries.count)
        .animation(BobeMotion.standard, value: self.store.conversationEnding)
    }

    @ViewBuilder
    var conversationEndingSection: some View {
        if self.store.conversationEnding {
            OverlayNotice(
                systemName: "moon.fill",
                tone: .info,
                message: L10n.tr("overlay.conversation.ending")
            )
            .overlayHitRegion()
            .transition(self.overlaySectionTransition)
        }
    }

    @ViewBuilder
    var softWarningSection: some View {
        if let warning = self.store.softWarning {
            OverlayNotice(
                systemName: "info",
                tone: .info,
                message: warning,
                onDismiss: { self.store.dismissSoftWarning() }
            )
            .overlayHitRegion()
            .transition(self.overlaySectionTransition)
        }
    }

    @ViewBuilder
    var screenUnavailableSection: some View {
        if self.store.screenUnavailableNotice {
            OverlayNotice(
                systemName: "eye.slash",
                tone: .info,
                message: L10n.tr("overlay.screen.unavailable.message"),
                detail: L10n.tr("setup.permissions.card.screen.denied"),
                actions: [
                    OverlayNoticeAction(id: "allow", title: L10n.tr("setup.permissions.grant"), isProminent: true) {
                        self.store.openScreenRecordingSettings()
                    },
                    OverlayNoticeAction(id: "turn-off", title: L10n.tr("overlay.screen.unavailable.turn_off")) {
                        Task { await self.store.setCapture(false) }
                    },
                ],
                onDismiss: { self.store.dismissScreenUnavailableNotice() }
            )
            .overlayHitRegion()
            .transition(self.overlaySectionTransition)
        }
    }

    @ViewBuilder
    var recoverySection: some View {
        ForEach(self.store.failedSendRecoveries) { recovery in
            OverlayNotice(
                systemName: recovery.canRetry ? "arrow.counterclockwise" : "exclamationmark",
                tone: .caution,
                message: recovery.content,
                detail: recovery.failureMessage,
                actions: recovery.canRetry
                    ? [
                        OverlayNoticeAction(id: "retry", title: L10n.tr("app.common.retry"), isProminent: true) {
                            Task { await self.store.retryFailedSendRecovery(recovery.id) }
                        },
                    ]
                    : [],
                onDismiss: { self.store.dismissFailedSendRecovery(recovery.id) }
            )
            .overlayHitRegion()
            .transition(self.overlaySectionTransition)
        }
    }

    @ViewBuilder
    var errorBannerSection: some View {
        if self.store.context.daemonError {
            OverlayNotice(
                systemName: "bolt.slash.fill",
                tone: .danger,
                message: self.store.repairIssue?.message ?? L10n.tr("overlay.error.daemon_disconnected"),
                actions: [
                    OverlayNoticeAction(id: "retry", title: L10n.tr("runtime.action.retry_connection"), isProminent: true) {
                        Task { await self.store.retryRuntimeConnection() }
                    },
                    OverlayNoticeAction(id: "repair", title: L10n.tr("runtime.action.repair")) {
                        self.store.repairRuntimeConnection()
                    },
                ]
            )
            .overlayHitRegion()
            .transition(self.overlaySectionTransition)
        } else if let error = self.store.errorMessage {
            OverlayNotice(
                systemName: "exclamationmark",
                tone: .danger,
                message: error,
                onDismiss: { self.store.dismissError() }
            )
            .overlayHitRegion()
            .transition(self.overlaySectionTransition)
        }
    }

    private func orbitAngle(_ angle: Double) -> Double {
        self.placement.satellitesBelow ? 360 - angle : angle
    }

    var avatarCluster: some View {
        ZStack {
            OverlayAvatar(
                mood: self.avatarMood,
                isEngaged: self.isAvatarEngaged,
                wantsAttention: self.wantsAttention,
                celebrationToken: self.celebrationToken,
                onTap: self.handleAvatarTap,
                onSecondaryTap: { event, view in
                    OverlayAppMenuController.shared.showContextMenu(for: event, in: view)
                }
            )
            OrbitSlot(
                angle: self.orbitAngle(AvatarMetrics.chatAngle),
                isShown: self.showChatToggleSatellite || self.focusedSatellite == .chat,
                index: 0
            ) {
                ChatToggleButton(
                    isActive: self.isChatVisible,
                    hasUnread: self.wantsAttention,
                    action: self.toggleChatManually
                )
                .focused(self.$focusedSatellite, equals: .chat)
            }
            OrbitSlot(
                angle: self.orbitAngle(AvatarMetrics.micAngle),
                isShown: self.showMicSatellite || self.focusedSatellite == .microphone,
                index: 1
            ) {
                MicButton()
                    .focused(self.$focusedSatellite, equals: .microphone)
            }
            OrbitSlot(
                angle: self.orbitAngle(AvatarMetrics.menuAngle),
                isShown: self.satellitesShown || self.focusedSatellite == .menu,
                index: 2
            ) {
                OverlaySettingsButton()
                    .focused(self.$focusedSatellite, equals: .menu)
            }
            if let tool = self.store.runningTools.first {
                ToolBadge(toolName: tool.toolName, count: self.store.runningTools.count)
                    .modifier(
                        OrbitPlacement(
                            angle: self.orbitAngle(AvatarMetrics.toolAngle),
                            radius: AvatarMetrics.cardSize / 2 - 1
                        )
                    )
                    .transition(.bobePop())
                    .allowsHitTesting(false)
            }
            if self.waitingReachoutCount > 0 {
                ReachoutPip(count: self.waitingReachoutCount)
                    .modifier(OrbitPlacement(angle: AvatarMetrics.pipAngle, radius: AvatarMetrics.cardSize / 2 - 3))
                    .transition(.bobePop())
                    .allowsHitTesting(false)
            }
        }
        .animation(BobeMotion.snappy, value: self.waitingReachoutCount)
        .animation(BobeMotion.snappy, value: self.store.runningTools.isEmpty)
        .frame(width: 0, height: 0)
        .position(
            x: AvatarMetrics.center.x,
            y: self.placement.satellitesBelow
                ? AvatarMetrics.clusterHeight - AvatarMetrics.center.y
                : AvatarMetrics.center.y
        )
        .frame(width: AvatarMetrics.clusterWidth, height: AvatarMetrics.clusterHeight)
        .zIndex(3)
    }
}

extension AvatarStatusBubble.Mode {
    var isPill: Bool {
        switch self {
        case .status, .silenced: true
        default: false
        }
    }
}

private struct OverlaySettingsButton: View {
    var body: some View {
        Button {
            BobeCommandRouter.shared.openSettings()
        } label: {
            SatelliteChrome(systemName: "gearshape.fill")
        }
        .buttonStyle(BobePressStyle(scale: 0.9))
        .accessibilityLabel(L10n.tr("overlay.settings.accessibility"))
        .accessibilityIdentifier("overlay.settings")
        .help(L10n.tr("overlay.settings.accessibility"))
    }
}

@MainActor
final class OverlayAppMenuController: NSObject {
    static let shared = OverlayAppMenuController()

    private func makeMenu() -> NSMenu {
        let menu = NSMenu()
        let settingsItem = NSMenuItem(
            title: L10n.tr("tray.settings"),
            action: #selector(self.openSettings),
            keyEquivalent: ","
        )
        settingsItem.target = self
        settingsItem.identifier = NSUserInterfaceItemIdentifier("overlay.app-menu.settings")
        menu.addItem(settingsItem)

        let hideItem = NSMenuItem(title: L10n.tr("tray.hide"), action: #selector(self.hideOverlay), keyEquivalent: "")
        hideItem.target = self
        hideItem.identifier = NSUserInterfaceItemIdentifier("overlay.app-menu.hide")
        menu.addItem(hideItem)
        menu.addItem(.separator())

        let quitItem = NSMenuItem(
            title: L10n.tr("tray.quit"),
            action: #selector(self.quitApp),
            keyEquivalent: "q"
        )
        quitItem.target = self
        quitItem.identifier = NSUserInterfaceItemIdentifier("overlay.app-menu.quit")
        menu.addItem(quitItem)
        return menu
    }

    func showContextMenu(for event: NSEvent, in view: NSView) {
        NSMenu.popUpContextMenu(self.makeMenu(), with: event, for: view)
    }

    @objc
    private func openSettings() {
        BobeCommandRouter.shared.openSettings()
    }

    @objc
    private func hideOverlay() {
        BobeCommandRouter.shared.hideOverlay()
    }

    @objc
    private func quitApp() {
        BobeCommandRouter.shared.quit()
    }
}
