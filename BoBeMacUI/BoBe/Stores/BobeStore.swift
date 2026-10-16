import AppKit
import CoreGraphics
import Foundation
import Observation
import OSLog

let bobeStoreLogger = Logger(subsystem: "com.bobe.app", category: "BobeStore")

@Observable @MainActor
final class BobeStore {
    static let shared = BobeStore()
    static let messageWindowCapacity = 200


    private(set) var context = BobeContext()
    var isReconnecting = false
    var repairIssue: RuntimeIssue?
    var isBackendFatal = false
    var hasConnectedOnce = false

    var proactiveSilenced = false


    private static let localeOverrideKey = "bobe.locale_override"
    static let supportedLocales = [
        "en-US", "el-GR", "zh-CN", "de-DE", "es-ES", "pt-BR", "ko-KR", "ja-JP", "fr-FR",
    ]
    private(set) var localeOverride: String =
        UserDefaults.standard.string(forKey: BobeStore.localeOverrideKey) ?? ""

    var stateType: BobeStateType {
        self.context.stateType
    }

    var isConnected: Bool {
        self.context.daemonConnected
    }

    var isCapturing: Bool {
        self.settingsStore.settings?.captureEnabled ?? false
    }

    var isThinking: Bool {
        self.context.thinking
    }

    var messages: [ChatMessage] {
        self.context.messages
    }

    private(set) var latestLiveBobeMessageId: String?
    private var presentedMessages: Set<String> = []

    func markResponsePresented(_ messageId: String) async {
        guard !self.presentedMessages.contains(messageId),
              let id = Self.turnIdentity(messageId) else { return }
        let selection = self.selection
        guard self.isCurrent(selection) else { return }
        do {
            try await DaemonClient(endpoint: selection.endpoint).markResponsePresented(id)
            guard self.isCurrent(selection) else { return }
            self.presentedMessages.formIntersection(Set(self.messages.map(\.id)))
            self.presentedMessages.insert(messageId)
        } catch {
            bobeStoreLogger.warning("Presentation receipt failed: \(error.localizedDescription, privacy: .public)")
        }
    }

    static func turnIdentity(_ messageId: String) -> UUID? {
        guard messageId.hasPrefix("msg_") || messageId.hasPrefix("voice_") else { return nil }
        let raw = String(messageId.dropFirst(messageId.hasPrefix("msg_") ? 4 : 6))
        if let id = UUID(uuidString: raw) { return id }
        guard raw.count == 32, raw.allSatisfy(\.isHexDigit) else { return nil }
        return UUID(uuidString: [
            String(raw.prefix(8)), String(raw.dropFirst(8).prefix(4)),
            String(raw.dropFirst(12).prefix(4)), String(raw.dropFirst(16).prefix(4)),
            String(raw.suffix(12)),
        ].joined(separator: "-"))
    }

    func recordLiveBobeMessage(_ messageId: String) {
        self.latestLiveBobeMessageId = messageId
    }

    var failedSendRecoveries: [FailedSendRecovery] {
        let selection = self.currentSelection()
        return self.context.failedSendRecoveries.filter { $0.selection == selection }
    }

    var errorMessage: String? {
        self.context.errorMessage
    }

    var toolExecutions: [ToolExecution] {
        self.context.toolExecutions
    }

    var runningTools: [ToolExecution] {
        self.context.toolExecutions.filter { $0.status == .running }
    }

    var softWarning: String? {
        self.context.softWarning
    }

    var conversationEnding: Bool {
        self.context.conversationEnding
    }

    var isInitialConnectionPending: Bool {
        !self.hasConnectedOnce && !self.context.daemonConnected && !self.isBackendFatal
    }

    var composerBlockReason: MessageComposerBlockReason? {
        if !self.context.daemonConnected {
            return self.hasConnectedOnce ? .reconnecting : .starting
        }
        if !self.context.acceptingUserMessages {
            return .thinking
        }
        return nil
    }


    let client = DaemonClient.shared
    var streamingMessage = ""
    var streamingMessageId: String?
    var lastMessageTimer: Task<Void, Never>?
    var conversationClearTask: Task<Void, Never>?
    var textDeltaFlushTask: Task<Void, Never>?
    var captureStartupTask: Task<Void, Never>?
    @ObservationIgnored var screenReadTask: Task<Void, Never>?
    var isReadingScreen = false
    var screenUnavailableNotice = false
    @ObservationIgnored var screenUnavailableNoticeShown = false
    var conversationReloadGeneration: UInt64 = 0
    var conversationMutationRevision: UInt64 = 0
    var streamMutationRevision: UInt64 = 0
    var eventSnapshotRecoveryActive = false
    private var backendObserverTask: Task<Void, Never>?
    @ObservationIgnored var reconnectStatusTask: Task<Void, Never>?
    @ObservationIgnored lazy var toolExecutionController = ToolExecutionController { [weak self] mutation in
        self?.updateState(mutation)
    }

    private let currentSelection: @MainActor () -> EndpointSelection
    private(set) var selection: EndpointSelection
    let settingsStore: SettingsStore
    private let sendRequest: @MainActor (EndpointSelection, String, UUID) async throws -> SendMessageResponse
    let conversationSnapshot: @MainActor (EndpointSelection) async throws -> ConversationSnapshotResponse
    private var endpointObserver: NSObjectProtocol?
    private var eventStreamGeneration: UInt64 = 0

    init(
        currentSelection: @escaping @MainActor () -> EndpointSelection = { EndpointStore.shared.selection },
        settingsStore: SettingsStore = .shared,
        sendRequest: @escaping @MainActor (EndpointSelection, String, UUID) async throws -> SendMessageResponse = {
            selection, content, requestID in
            try await DaemonClient(endpoint: selection.endpoint).sendMessage(content, requestId: requestID)
        },
        conversationSnapshot: @escaping @MainActor (EndpointSelection) async throws -> ConversationSnapshotResponse = {
            try await DaemonClient(endpoint: $0.endpoint).getCurrentConversation()
        }
    ) {
        self.currentSelection = currentSelection
        self.selection = currentSelection()
        self.settingsStore = settingsStore
        self.sendRequest = sendRequest
        self.conversationSnapshot = conversationSnapshot
        self.endpointObserver = NotificationCenter.default.addObserver(
            forName: .bobeEndpointIdentityChanged, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self, self.selection != self.currentSelection() else { return }
                let wasObserving = self.backendObserverTask != nil
                self.synchronizeEndpoint()
                if wasObserving { self.connect() }
            }
        }
    }

    func synchronizeEndpoint() {
        let selection = self.currentSelection()
        guard self.selection != selection else { return }
        self.selection = selection
        self.conversationReloadGeneration &+= 1
        self.textDeltaFlushTask?.cancel()
        self.textDeltaFlushTask = nil
        self.lastMessageTimer?.cancel()
        self.conversationClearTask?.cancel()
        self.captureStartupTask?.cancel()
        self.streamingMessage = ""
        self.streamingMessageId = nil
        self.latestLiveBobeMessageId = nil
        self.presentedMessages = []
        self.eventSnapshotRecoveryActive = false
        self.context = BobeContext()
        self.hasConnectedOnce = false
        self.isBackendFatal = false
        self.repairIssue = nil
    }

    func isCurrent(_ selection: EndpointSelection) -> Bool {
        self.selection == selection && self.currentSelection() == selection
    }

    func observeBackendState() {
        guard self.backendObserverTask == nil else { return }
        self.backendObserverTask = Task { [weak self] in
            let states = await BackendService.shared.stateStream
            for await state in states {
                guard !Task.isCancelled else { return }
                await MainActor.run { [weak self] in
                    switch state {
                    case .fatal:
                        self?.cancelReconnectStatusTransition()
                        self?.isBackendFatal = true
                        self?.isReconnecting = false
                        self?.updateState { ctx in
                            ctx.daemonConnected = false
                            ctx.daemonError = true
                        }
                    case .ready:
                        self?.cancelReconnectStatusTransition()
                        self?.isBackendFatal = false
                        self?.isReconnecting = false
                        self?.updateState { $0.daemonError = false }
                    case .crashed:
                        self?.scheduleReconnectStatusTransition()
                    default:
                        break
                    }
                }
            }
        }
    }


    func connect() {
        self.synchronizeEndpoint()
        let selection = self.selection
        self.eventStreamGeneration &+= 1
        let generation = self.eventStreamGeneration
        self.observeBackendState()
        Task { [weak self] in
            guard let self, self.acceptsEventStream(generation: generation, selection: selection) else { return }
            await self.client.connectSSE(
                generation: generation,
                selection: selection,
                onEvent: { [weak self] bundle in
                    await self?.processEvent(bundle, generation: generation, selection: selection) ?? false
                },
                onConnectionChange: { [weak self] state in
                    await self?.processConnectionChange(state, generation: generation, selection: selection)
                }
            )
        }
    }

    private func processEvent(
        _ bundle: StreamBundle, generation: UInt64, selection: EndpointSelection
    ) async -> Bool {
        guard self.acceptsEventStream(generation: generation, selection: selection) else { return false }
        return await self.processBundle(bundle)
    }

    private func processConnectionChange(
        _ state: RuntimeConnectionState, generation: UInt64, selection: EndpointSelection
    ) async {
        guard self.acceptsEventStream(generation: generation, selection: selection) else { return }
        await self.handleConnectionChange(state)
    }

    private func acceptsEventStream(generation: UInt64, selection: EndpointSelection) -> Bool {
        self.eventStreamGeneration == generation && self.isCurrent(selection)
    }

    func suspendEventStream() async {
        self.eventStreamGeneration &+= 1
        await self.client.disconnectSSE(generation: self.eventStreamGeneration)
    }

    func resumeEventStream() {
        guard !self.isBackendFatal else { return }
        self.connect()
    }

    func disconnect() {
        self.conversationReloadGeneration &+= 1
        self.textDeltaFlushTask?.cancel()
        self.lastMessageTimer?.cancel()
        self.conversationClearTask?.cancel()
        self.captureStartupTask?.cancel()
        self.eventSnapshotRecoveryActive = false
        self.backendObserverTask?.cancel()
        self.backendObserverTask = nil
        self.reconnectStatusTask?.cancel()
        self.eventStreamGeneration &+= 1
        let generation = self.eventStreamGeneration
        Task {
            await self.client.disconnectSSE(generation: generation)
        }
    }

    func beginShutdown() {
        self.updateState { $0.shuttingDown = true }
    }

    func updateLocale(_ newOverride: String) {
        self.localeOverride = newOverride
        if newOverride.isEmpty {
            UserDefaults.standard.removeObject(forKey: BobeStore.localeOverrideKey)
        } else {
            UserDefaults.standard.set(newOverride, forKey: BobeStore.localeOverrideKey)
        }
        L10n.setLocaleOverride(newOverride.isEmpty ? nil : newOverride)
    }

    func applyPersistedLocale() {
        L10n.setLocaleOverride(self.localeOverride.isEmpty ? nil : self.localeOverride)
    }


    func retryRuntimeConnection() async {
        do {
            _ = try await EndpointAttachService.shared.attachConfiguredEndpoint()
            self.repairIssue = nil
            self.isReconnecting = false
            self.updateState { $0.daemonError = false }
            self.connect()
        } catch {
            let issue = RuntimeIssue(error)
            self.repairIssue = issue
            if issue.primaryAction != .retryConnection {
                SetupWindowManager.shared.showRuntimeRepair(issue: issue)
            }
        }
    }

    func repairRuntimeConnection() {
        let issue = self.repairIssue ?? RuntimeIssue(AttachError.unreachable)
        SetupWindowManager.shared.showRuntimeRepair(issue: issue)
    }

    func dismissError() {
        self.updateState { ctx in
            ctx.errorMessage = nil
            ctx.daemonError = false
        }
    }

    func dismissSoftWarning() {
        self.updateState { $0.softWarning = nil }
    }

    func toggleProactiveSilence() {
        self.proactiveSilenced.toggle()
        if self.proactiveSilenced {
            VoicePipeline.shared.interrupt()
        }
    }

    func surfaceWarning(_ message: String) {
        self.updateState { ctx in
            ctx.errorMessage = message
        }
    }

    func toggleCapture() async -> Bool {
        await self.setCapture(!self.isCapturing)
    }

    @discardableResult
    func setCapture(_ enabled: Bool) async -> Bool {
        guard await self.settingsStore.setCaptureEnabled(enabled) else {
            bobeStoreLogger.error("Screen awareness change failed")
            return self.isCapturing
        }
        if enabled, !CGPreflightScreenCaptureAccess() {
            CGRequestScreenCaptureAccess()
        }
        if !enabled {
            self.screenUnavailableNotice = false
        }
        return enabled
    }

    func appendUserVoiceMessage(_ content: String) {
        self.cancelConversationClear()
        let userMessage = ChatMessage(
            id: "voice-\(Int(Date.now.timeIntervalSince1970 * 1000))",
            sender: .user,
            content: content,
            isPending: false
        )
        self.updateState { ctx in
            ctx.errorMessage = nil
            ctx.conversationEnding = false
            ctx.messages.append(userMessage)
        }
    }

    func sendMessage(_ content: String, requestId: UUID = UUID()) async {
        self.synchronizeEndpoint()
        let selection = self.selection
        self.cancelConversationClear()
        let userMessage = ChatMessage(
            id: "user-\(requestId.uuidString.lowercased())",
            sender: .user,
            content: content,
            isPending: true
        )
        self.updateState { ctx in
            ctx.errorMessage = nil
            ctx.conversationEnding = false
            ctx.messages.append(userMessage)
        }

        do {
            let response = try await self.sendRequest(selection, content, requestId)
            guard self.isCurrent(selection) else { return }
            self.updateState { ctx in
                if response.replayed {
                    Self.removeMessage(userMessage.id, messages: &ctx.messages)
                } else {
                    Self.markMessageSent(userMessage.id, messages: &ctx.messages)
                }
                ctx.acceptingUserMessages = true
            }
            if response.replayed {
                await self.reloadCanonicalConversation()
            }
        } catch {
            guard self.isCurrent(selection) else { return }
            let unsafeReplayMessage = Self.unsafeReplayMessage(error)
            bobeStoreLogger.error("sendMessage failed: \(error.localizedDescription)")
            self.updateState { ctx in
                Self.removeMessage(userMessage.id, messages: &ctx.messages)
                ctx.failedSendRecoveries.append(
                    FailedSendRecovery(
                        selection: selection,
                        id: userMessage.id,
                        content: content,
                        requestId: requestId,
                        canRetry: unsafeReplayMessage == nil,
                        failureMessage: unsafeReplayMessage
                    )
                )
                if unsafeReplayMessage == nil {
                    ctx.errorMessage = error.localizedDescription
                    ctx.daemonError = false
                }
                ctx.acceptingUserMessages = true
            }
        }
    }

    private static func unsafeReplayMessage(_ error: any Error) -> String? {
        if case let DaemonError.httpError(_, message, code) = error,
           code == "REQUEST_REPLAY_UNSAFE" {
            return message
        }
        return nil
    }

    func dismissFailedSendRecovery(_ recoveryId: String) {
        self.updateState { ctx in
            ctx.failedSendRecoveries.removeAll { $0.id == recoveryId }
        }
    }

    func retryFailedSendRecovery(_ recoveryId: String) async {
        self.synchronizeEndpoint()
        guard let recovery = self.context.failedSendRecoveries.first(where: { $0.id == recoveryId }) else {
            return
        }
        guard recovery.canRetry, self.isCurrent(recovery.selection) else { return }

        self.updateState { ctx in
            ctx.failedSendRecoveries.removeAll { $0.id == recoveryId }
        }

        await self.sendMessage(recovery.content, requestId: recovery.requestId)
    }

    static func markMessageSent(_ messageId: String, messages: inout [ChatMessage]) {
        self.updateMessage(messageId, messages: &messages) { message in
            message.isStreaming = false
            message.isPending = false
        }
    }

    static func removeMessage(_ messageId: String, messages: inout [ChatMessage]) {
        messages.removeAll { $0.id == messageId }
    }

    @discardableResult
    static func updateMessage(
        _ messageId: String,
        messages: inout [ChatMessage],
        mutate: (inout ChatMessage) -> Void
    ) -> Bool {
        guard let idx = messages.firstIndex(where: { $0.id == messageId }) else {
            return false
        }

        mutate(&messages[idx])
        return true
    }

    static func trimMessages(_ messages: inout [ChatMessage], capacity: Int = messageWindowCapacity) {
        guard capacity >= 0, messages.count > capacity else { return }
        let overflow = messages.count - capacity
        let protectedIds = Set(messages.filter { $0.isPending || $0.isStreaming }.map(\.id))
        let removable = messages.indices.filter { !protectedIds.contains(messages[$0].id) }
        for index in removable.prefix(overflow).reversed() {
            messages.remove(at: index)
        }
    }

    func updateState(_ block: (inout BobeContext) -> Void) {
        var ctx = self.context
        block(&ctx)
        Self.trimMessages(&ctx.messages)
        if ctx.messages != self.context.messages {
            self.conversationMutationRevision &+= 1
        }
        ctx.stateType = deriveStateType(from: ctx)
        self.context = ctx
    }
}
