@testable import BoBe
import Foundation
import Testing

@Suite("UI foundations", .serialized)
struct UIFoundationTests {
    @Test
    func balancedSetupObservesEveryTwoMinutes() {
        #expect(ProactivityLevel.balanced.settings.captureIntervalSeconds == 120)
        #expect(ProactivityLevel.balanced.settings.captureEnabled)
        #expect(!ProactivityLevel.quiet.settings.captureEnabled)
    }

    @Test
    func setupCountsOnlyTheStepsOnThePersonsPath() {
        var route = SetupRoute()
        #expect(route.stepsRemaining(after: .welcome) + 1 == 10)
        #expect(route.step(after: .engineChoice) == .providerSetup)

        route.engineChoice = .local
        #expect(route.step(after: .engineChoice) == .localSetup)
        #expect(route.stepsRemaining(after: .welcome) + 1 == 10)

        route.initializationState = .personalizationRequired
        #expect(route.step(after: .runtimeLocation) == .personalize)
        #expect(route.stepsRemaining(after: .welcome) + 1 == 8)

        route.initializationState = .ready
        #expect(route.step(after: .runtimeLocation) == .permissions)
        #expect(route.stepsRemaining(after: .welcome) + 1 == 6)

        #expect(SetupRoute(initialStep: .physicalBoBe).stepsRemaining(after: .physicalBoBe) == 1)
        #expect(SetupRoute(initialStep: .voiceSetup).step(after: .voiceSetup) == nil)
        #expect(SetupRoute(completesAfterRuntimeRepair: true).step(after: .runtimeLocation) == nil)
    }

    @Test
    @MainActor
    func settingsWindowUsesProductiveInitialSize() {
        let size = SettingsWindowManager.initialSize(
            for: .init(x: 0, y: 0, width: 1512, height: 945)
        )
        #expect(size.width >= 1200)
        #expect(size.height >= 800)

        let constrained = SettingsWindowManager.initialSize(
            for: .init(x: 0, y: 0, width: 1024, height: 640)
        )
        #expect(constrained.width <= 984)
        #expect(constrained.height <= 600)
    }

    @Test
    @MainActor
    func settingsWindowClampsRestoredFrameToVisibleDisplay() {
        let screen = CGRect(x: 100, y: 50, width: 1200, height: 800)
        let frame = SettingsWindowManager.clampedFrame(
            CGRect(x: -400, y: 900, width: 1600, height: 300),
            to: screen,
            minimumSize: CGSize(width: 760, height: 520)
        )

        #expect(frame == CGRect(x: 100, y: 330, width: 1200, height: 520))
        #expect(screen.contains(frame))
    }

    @Test
    @MainActor
    func settingsWindowPreservesValidRestoredFrame() {
        let screen = CGRect(x: 0, y: 0, width: 1512, height: 945)
        let restored = CGRect(x: 180, y: 120, width: 1100, height: 720)

        #expect(
            SettingsWindowManager.clampedFrame(
                restored,
                to: screen,
                minimumSize: CGSize(width: 760, height: 520)
            ) == restored
        )
    }

    @Test
    @MainActor
    func settingsSearchMatchesLocalizedPanelConceptsAndAllTokens() {
        #expect(SettingsCategory.voice.matchesSearch("speech recognition"))
        #expect(SettingsCategory.behavior.matchesSearch("screen awareness"))
        #expect(SettingsCategory.mcpServers.matchesSearch("tools services"))
        #expect(SettingsCategory.runtime.matchesSearch("remote address"))
        #expect(!SettingsCategory.appearance.matchesSearch("speech recognition"))
        #expect(SettingsCategory.appearance.matchesSearch("   "))
    }

    @Test
    @MainActor
    func commandTitlesReflectOnboardingPolicy() {
        let defaults = UserDefaults.standard
        let key = SetupWindowManager.onboardingCompletedKey
        let authoritiesKey = SetupWindowManager.completedAuthoritiesKey
        let previous = defaults.object(forKey: key)
        let previousAuthorities = defaults.object(forKey: authoritiesKey)
        defer {
            if let previous {
                defaults.set(previous, forKey: key)
            } else {
                defaults.removeObject(forKey: key)
            }
            if let previousAuthorities {
                defaults.set(previousAuthorities, forKey: authoritiesKey)
            } else {
                defaults.removeObject(forKey: authoritiesKey)
            }
        }

        defaults.set([], forKey: authoritiesKey)
        defaults.set(false, forKey: key)
        #expect(
            BobeCommandRouter.shared.overlayTitle
                == L10n.tr("app.setup_incomplete.retry")
        )
        defaults.set(true, forKey: key)
        if let authority = EndpointStore.shared.runtimeAuthorityId {
            defaults.set([authority.uuidString], forKey: authoritiesKey)
        }
        #expect(
            [L10n.tr("tray.show"), L10n.tr("tray.hide")]
                .contains(BobeCommandRouter.shared.overlayTitle)
        )
    }

    @Test
    @MainActor
    func onboardingCompletionIsScopedToCanonicalRuntimeAuthority() throws {
        let defaults = try #require(EphemeralUserDefaults(prefix: "UIFoundationTests.onboarding"))

        let completedAuthority = UUID()
        defaults.set(true, forKey: SetupWindowManager.onboardingCompletedKey)
        defaults.set(
            [completedAuthority.uuidString],
            forKey: SetupWindowManager.completedAuthoritiesKey
        )

        #expect(
            SetupWindowManager.isOnboardingCompleted(
                defaults: defaults,
                authority: completedAuthority
            )
        )
        #expect(
            !SetupWindowManager.isOnboardingCompleted(
                defaults: defaults,
                authority: UUID()
            )
        )
        #expect(
            SetupWindowManager.isOnboardingCompleted(
                defaults: defaults,
                authority: UUID(uuidString: completedAuthority.uuidString.lowercased())
            )
        )
    }

    @Test
    @MainActor
    func docklessAppProvidesPersistentQuitMenu() {
        let defaults = UserDefaults.standard
        let onboardingKey = SetupWindowManager.onboardingCompletedKey
        let authoritiesKey = SetupWindowManager.completedAuthoritiesKey
        let previousOnboardingValue = defaults.object(forKey: onboardingKey)
        let previousAuthorities = defaults.object(forKey: authoritiesKey)
        defaults.set(
            EndpointStore.shared.runtimeAuthorityId.map { [$0.uuidString] } ?? [],
            forKey: authoritiesKey
        )
        defaults.set(true, forKey: onboardingKey)
        defer {
            if let previousOnboardingValue {
                defaults.set(previousOnboardingValue, forKey: onboardingKey)
            } else {
                defaults.removeObject(forKey: onboardingKey)
            }
            if let previousAuthorities {
                defaults.set(previousAuthorities, forKey: authoritiesKey)
            } else {
                defaults.removeObject(forKey: authoritiesKey)
            }
        }

        let controller = BobeStatusItemController()
        controller.setup()
        defer { controller.tearDown() }

        #expect(controller.statusItem?.button?.image != nil)
        let accessibilityStatus = controller.statusItem?.button?
            .accessibilityValue() as? String
        #expect(accessibilityStatus?.isEmpty == false)
        #expect(controller.statusItem?.button?.image?.isTemplate == true)
        #expect(controller.statusItem?.menu != nil)
        let titles = controller.statusItem?.menu?.items.map(\.title) ?? []
        #expect(titles.last == L10n.tr("tray.quit"))
        #expect(titles.contains(L10n.tr("tray.settings")))
        #expect(titles.count { !$0.isEmpty } == 4)
    }

    @Test
    @MainActor
    func unsavedCoordinatorDefersAndDiscardsTransition() {
        let coordinator = SettingsEditCoordinator.shared
        var discarded = false
        var transitioned = false
        coordinator.register(
            isDirty: true,
            save: { true },
            discard: { discarded = true }
        )
        coordinator.requestTransition { transitioned = true }
        #expect(coordinator.showsConfirmation)
        #expect(!transitioned)
        coordinator.discardAndContinue()
        #expect(discarded)
        #expect(transitioned)
        #expect(!coordinator.showsConfirmation)
    }

    @Test
    @MainActor
    func unsavedCoordinatorCancelKeepsDraftAndDestination() {
        let coordinator = SettingsEditCoordinator.shared
        var transitioned = false
        coordinator.register(isDirty: true, save: { true }, discard: {})
        coordinator.requestTransition { transitioned = true }
        coordinator.cancelTransition()
        #expect(!transitioned)
        #expect(coordinator.isDirty)
    }

    @Test
    func saveResponsePreservesNewerText() {
        #expect(reconcileSavedDraft(current: "later edit", submitted: "sent", saved: "sent") == "later edit")
        #expect(reconcileSavedDraft(current: "sent", submitted: "sent", saved: "normalized") == "normalized")
    }

    @Test
    func goalLifecycleMergesStatusWithoutErasingDirtyProse() {
        var original = GoalDraft()
        original.notes = "saved"
        var draft = original
        draft.notes = "unsaved"
        var completed = original
        completed.status = .completed
        let merged = draft.reconciled(submitted: original, saved: completed)
        #expect(merged.notes == "unsaved")
        #expect(merged.status == .completed)
        #expect(merged != completed)
        var clean = original.reconciled(submitted: original, saved: completed)
        clean.notes = "new note"
        #expect(clean.status == .completed)
    }

    @Test
    func goalSavePreservesFieldsChangedDuringRequest() {
        var submitted = GoalDraft()
        submitted.notes = "sent notes"
        var latest = submitted
        latest.notes = "later notes"
        var saved = submitted
        saved.title = "normalized title"
        let merged = latest.reconciled(submitted: submitted, saved: saved)
        #expect(merged.notes == "later notes")
        #expect(merged.title == "normalized title")
    }

    @Test
    func externalProviderRestartRequiresExactCommittedInput() {
        let selection = EndpointSelection(
            generation: 1, profileID: UUID(), authorityID: UUID(), endpoint: .unavailable
        )
        let request = ProviderActivationRequest(
            kind: .openAI, apiKey: "original", chatModel: "chat", goalsModel: "batch",
            consolidateModel: "batch"
        )
        let pending = PendingProviderActivation(
            selection: selection, profileID: "openai", request: request, previousIncarnation: UUID()
        )
        #expect(pending.matches(selection: selection, profileID: "openai", request: request))
        let edited = ProviderActivationRequest(
            kind: .openAI, apiKey: "corrected", chatModel: "chat", goalsModel: "batch",
            consolidateModel: "batch"
        )
        #expect(!pending.matches(selection: selection, profileID: "openai", request: edited))
        #expect(!pending.matches(selection: selection, profileID: "anthropic", request: request))
        let revisited = EndpointSelection(
            generation: 3, profileID: selection.profileID, authorityID: selection.authorityID, endpoint: .unavailable
        )
        #expect(!pending.matches(selection: revisited, profileID: "openai", request: request))
        var probeRetry = pending
        probeRetry.previousIncarnation = nil
        #expect(probeRetry.matches(selection: selection, profileID: "openai", request: request))
        #expect(probeRetry.previousIncarnation == nil)
    }

    @Test
    func loadedProviderModelsAreNotMistakenForUserSelectionDefaults() {
        func route(_ model: String) -> WorkloadRouteWire {
            WorkloadRouteWire(profile: "anthropic", model: model, reasoningEffort: nil)
        }
        let response = ProvidersResponse(
            profiles: [ProviderProfileWire(id: "anthropic", kind: .anthropic)],
            routes: ProviderRoutesWire(
                chat: route("custom-chat"), goals: route("custom-batch"),
                consolidate: route("custom-batch")
            ),
            activeProfiles: [ProviderProfileWire(id: "anthropic", kind: .anthropic)],
            activeRoutes: ProviderRoutesWire(
                chat: route("running-chat"), goals: route("custom-batch"),
                consolidate: route("custom-batch")
            ),
            restartRequired: false
        )
        var fields = ProviderConfigurationFields()
        fields.load(response)
        #expect(fields.kind == .anthropic)
        #expect(fields.chat == "custom-chat")
        #expect(fields.batch == "custom-batch")
        fields.selectKind(.anthropic)
        #expect(fields.chat == "custom-chat")
        fields.selectKind(.openAI)
        #expect(fields.chat.isEmpty)
        #expect(response.isPending(.chat))
        #expect(!response.isPending(.goals))
        #expect(!response.canRemove(response.profiles[0]))
        fields.load(response)
        #expect(fields.chat == "custom-chat")
    }

    @Test
    @MainActor
    func failedMessagesCannotBeRetriedOnAnotherAuthority() async {
        var selection = EndpointSelection(
            generation: 1, profileID: UUID(), authorityID: UUID(), endpoint: .unavailable
        )
        var sends = 0
        let store = BobeStore(currentSelection: { selection }, sendRequest: { _, _, _ in
            sends += 1
            throw URLError(.notConnectedToInternet)
        })
        await store.sendMessage("private to A")
        let recoveryID = store.failedSendRecoveries.first?.id ?? "missing"
        #expect(store.failedSendRecoveries.count == 1)
        selection = EndpointSelection(
            generation: 2, profileID: UUID(), authorityID: UUID(), endpoint: .unavailable
        )
        await store.retryFailedSendRecovery(recoveryID)
        #expect(sends == 1)
        #expect(store.failedSendRecoveries.isEmpty)
        #expect(store.messages.isEmpty)
    }

    @Test
    @MainActor
    func lateFailedSendDoesNotRecreatePreviousAuthorityRecovery() async {
        var selection = EndpointSelection(
            generation: 1, profileID: UUID(), authorityID: UUID(), endpoint: .unavailable
        )
        var resume: CheckedContinuation<Void, Never>?
        let store = BobeStore(currentSelection: { selection }, sendRequest: { _, _, _ in
            await withCheckedContinuation { resume = $0 }
            throw URLError(.networkConnectionLost)
        })
        let send = Task { await store.sendMessage("old authority") }
        while resume == nil { await Task.yield() }
        selection = EndpointSelection(
            generation: 2, profileID: UUID(), authorityID: UUID(), endpoint: .unavailable
        )
        store.synchronizeEndpoint()
        resume?.resume()
        await send.value
        #expect(store.failedSendRecoveries.isEmpty)
        #expect(store.messages.isEmpty)
        #expect(store.errorMessage == nil)
    }

    @Test
    @MainActor
    func lateSuccessfulSendCannotLockNewAuthorityComposer() async {
        var selection = EndpointSelection(
            generation: 1, profileID: UUID(), authorityID: UUID(), endpoint: .unavailable
        )
        var resume: CheckedContinuation<Void, Never>?
        let store = BobeStore(currentSelection: { selection }, sendRequest: { _, _, _ in
            await withCheckedContinuation { resume = $0 }
            return SendMessageResponse(messageId: "old", replayed: false)
        })
        let send = Task { await store.sendMessage("old authority") }
        while resume == nil { await Task.yield() }
        selection = EndpointSelection(
            generation: 2, profileID: UUID(), authorityID: UUID(), endpoint: .unavailable
        )
        store.synchronizeEndpoint()
        resume?.resume()
        await send.value
        #expect(store.context.acceptingUserMessages)
        #expect(store.messages.isEmpty)
    }

    @Test
    @MainActor
    func aQueuedMessageAndAnActiveTurnDoNotDisableTheComposer() async {
        let selection = EndpointSelection(generation: 1, profileID: UUID(), authorityID: UUID(), endpoint: .unavailable)
        let store = BobeStore(currentSelection: { selection }, sendRequest: { _, _, _ in
            SendMessageResponse(messageId: "queued", replayed: false)
        })
        store.updateState {
            $0.daemonConnected = true
            $0.acceptingUserMessages = true
            $0.thinking = true
        }
        #expect(store.composerBlockReason == nil)
        await store.sendMessage("The next question")
        #expect(store.context.acceptingUserMessages)
        #expect(store.composerBlockReason == nil)
    }

    @Test
    @MainActor
    func presentationReceiptsUseTheActualCompactTurnWireIdentity() {
        let id = UUID(uuidString: "4cd96cab-bf3e-44af-96ff-07954f557030")
        #expect(BobeStore.turnIdentity("msg_4cd96cabbf3e44af96ff07954f557030") == id)
        #expect(BobeStore.turnIdentity("voice_4cd96cabbf3e44af96ff07954f557030") == id)
        #expect(BobeStore.turnIdentity("msg_4cd96cab-bf3e-44af-96ff-07954f557030") == id)
        #expect(BobeStore.turnIdentity("user-4cd96cabbf3e44af96ff07954f557030") == nil)
    }

    @Test
    @MainActor
    func settingsDebouncerFlushWaitsForOlderWriteBeforeLatest() async {
        let debouncer = SettingsDebouncer()
        var resume: CheckedContinuation<Void, Never>?
        var writes: [Int] = []
        debouncer.debounce {
            writes.append(1)
            await withCheckedContinuation { resume = $0 }
        }
        let first = Task { await debouncer.flush() }
        while resume == nil { await Task.yield() }
        debouncer.debounce { writes.append(2) }
        let second = Task { await debouncer.flush() }
        await Task.yield()
        #expect(writes == [1])
        resume?.resume()
        await first.value
        await second.value
        #expect(writes == [1, 2])
    }

    @Test
    @MainActor
    func cancelingQueuedSettingsDoesNotRunThemOnNextFlush() async {
        let debouncer = SettingsDebouncer()
        var writes = 0
        debouncer.debounce { writes += 1 }
        debouncer.cancelPendingSave()
        await debouncer.flush()
        #expect(writes == 0)
    }

    @Test
    func ambientMessageDwellHasNoReadingTimeCap() {
        let short = AmbientMessageTiming.dwellDuration(for: "Okay.")
        let medium = AmbientMessageTiming.dwellDuration(
            for: "I found the issue and updated the relevant settings. The complete explanation remains available so you can read it without rushing."
        )
        let long = AmbientMessageTiming.dwellDuration(for: String(repeating: "word ", count: 200))

        #expect(short == AmbientMessageTiming.minimumDwellSeconds)
        #expect(medium > short)
        #expect(long > medium)
    }

    @Test
    func onlyBriefProactiveMessagesAutoDismiss() {
        let requested = ChatMessage(
            sender: .bobe,
            content: "Requested answer",
            responseOrigin: .userInitiated
        )
        let proactive = ChatMessage(
            sender: .bobe,
            content: "A brief proactive note",
            responseOrigin: .proactive
        )
        let longProactive = ChatMessage(
            sender: .bobe,
            content: String(repeating: "word ", count: 61),
            responseOrigin: .proactive
        )

        #expect(!AmbientMessageTiming.shouldAutoDismiss(requested))
        #expect(AmbientMessageTiming.shouldAutoDismiss(proactive))
        #expect(!AmbientMessageTiming.shouldAutoDismiss(longProactive))
    }

    @Test
    func emptyAssistantPlaceholderDoesNotCreateConversationTrace() {
        let placeholder = ChatMessage(sender: .bobe, content: "")
        let streaming = ChatMessage(sender: .bobe, content: "", isStreaming: true)

        #expect(!placeholder.belongsInConversationTrace)
        #expect(streaming.belongsInConversationTrace)
    }
}
