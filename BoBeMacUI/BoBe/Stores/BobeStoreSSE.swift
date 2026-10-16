import Foundation

enum CanonicalConversationReload: Equatable {
    case applied
    case skipped
    case unavailable
}

extension BobeStore {
    @discardableResult
    func processBundle(_ bundle: StreamBundle) async -> Bool {
        do {
            return try await self.apply(bundle)
        } catch {
            bobeStoreLogger.error(
                "Malformed \(bundle.type.rawValue, privacy: .public) event left unacknowledged: \(error.localizedDescription, privacy: .public)"
            )
            return false
        }
    }

    private func apply(_ bundle: StreamBundle) async throws -> Bool {
        switch bundle.type {
        case .indicator:
            let payload = try bundle.payload.decode(as: IndicatorPayload.self)
            if payload.indicator == .idle {
                self.eventSnapshotRecoveryActive = false
            }
            self.handleIndicator(payload)
        case .textDelta:
            let payload = try bundle.payload.decode(as: TextDeltaPayload.self)
            if !self.eventSnapshotRecoveryActive || payload.done {
                self.handleTextDelta(payload, messageId: bundle.messageId)
                if payload.done {
                    self.eventSnapshotRecoveryActive = false
                }
            }
        case .toolCallStart:
            let payload = try bundle.payload.decode(as: ToolCallStartPayload.self)
            if !self.eventSnapshotRecoveryActive {
                self.toolExecutionController.start(payload)
            }
        case .toolCallComplete:
            let payload = try bundle.payload.decode(as: ToolCallCompletePayload.self)
            if !self.eventSnapshotRecoveryActive {
                self.toolExecutionController.complete(payload)
            }
        case .conversationClosed:
            self.handleConversationClosed(try bundle.payload.decode(as: ConversationClosedPayload.self))
        case .conversationChanged:
            let payload = try bundle.payload.decode(as: ConversationChangedPayload.self)
            bobeStoreLogger.info("Conversation \(payload.conversationId) changed; loading the current conversation")
            switch await self.reloadCanonicalConversation() {
            case .applied:
                self.eventSnapshotRecoveryActive = false
            case .skipped:
                break
            case .unavailable:
                return false
            }
        case .snapshotRequired:
            let payload = try bundle.payload.decode(as: SnapshotRequiredPayload.self)
            guard payload.snapshotRequired else { return false }
            bobeStoreLogger.warning("Event stream snapshot required: \(payload.reason)")
            self.eventSnapshotRecoveryActive = true
            guard await self.reloadCanonicalConversation(replacingActiveStream: true) == .applied else {
                bobeStoreLogger.error("Event stream snapshot recovery failed")
                return false
            }
            self.eventSnapshotRecoveryActive = false
        case .error:
            let payload = try bundle.payload.decode(as: ErrorPayload.self)
            self.eventSnapshotRecoveryActive = false
            self.handleErrorPayload(payload, messageId: bundle.messageId)
        case .bodyRequest:
            self.handleBodyRequest(try bundle.payload.decode(as: BodyRequestPayload.self))
        case .unknown:
            break
        }
        return true
    }

    private func handleIndicator(_ payload: IndicatorPayload) {
        let indicator = payload.indicator

        self.updateState { ctx in
            switch indicator {
            case .idle:
                ctx.thinking = false
                ctx.speaking = false
                ctx.acceptingUserMessages = true
            case .thinking:
                ctx.thinking = true
                ctx.speaking = false
            case .streaming:
                let hasVisibleText = self.hasVisibleGlyphs(self.streamingMessage) || self.hasVisibleGlyphs(ctx.currentMessage)
                ctx.thinking = !hasVisibleText
                ctx.speaking = hasVisibleText
            case .unknown:
                break
            }
            if indicator != .unknown {
                ctx.errorMessage = nil
                ctx.daemonError = false
            }
        }
        if indicator == .idle,
           self.streamingMessageId != nil || !self.context.currentMessage.isEmpty {
            Task { @MainActor [weak self] in
                await self?.reloadCanonicalConversation(replacingActiveStream: true)
            }
        }
    }

    private func handleErrorPayload(_ payload: ErrorPayload, messageId: String) {
        if !payload.isTriggerError {
            self.finalizeStreamingMessageAfterError(messageId: messageId)
        }
        if payload.code == "turn_cancelled" {
            self.updateState {
                $0.thinking = false
                $0.speaking = false
                $0.acceptingUserMessages = true
                $0.errorMessage = nil
                $0.daemonError = false
            }
            return
        }

        if payload.recoverable {
            if payload.isTriggerError {
                bobeStoreLogger.warning("Trigger soft warning [\(payload.sourceLabel)]: \(payload.message)")
            } else {
                bobeStoreLogger.warning("Recoverable chat error [\(payload.sourceLabel)]: \(payload.message)")
            }
            self.updateState { $0.softWarning = payload.message }
            return
        }
        bobeStoreLogger.error("Daemon error [\(payload.sourceLabel)]: \(payload.message)")
        self.updateState { ctx in
            ctx.errorMessage = payload.message
            ctx.daemonError = true
        }
    }

    private func finalizeStreamingMessageAfterError(messageId: String) {
        guard self.streamingMessageId == messageId else { return }
        self.flushStreamingToUI(messageId: messageId)
        self.finalizeStreamingMessage(isComplete: false)
    }

    private func handleTextDelta(_ payload: TextDeltaPayload, messageId: String) {
        self.streamMutationRevision &+= 1
        self.cancelConversationClear()
        if self.streamingMessageId != messageId {
            self.streamingMessage = ""
            self.streamingMessageId = messageId
            self.textDeltaFlushTask?.cancel()
            self.textDeltaFlushTask = nil
        }

        if payload.done {
            self.streamingMessage = payload.delta
            self.flushStreamingToUI(messageId: messageId)
            self.finalizeStreamingMessage()
            return
        }

        self.streamingMessage += payload.delta

        if self.textDeltaFlushTask == nil {
            self.textDeltaFlushTask = Task { @MainActor [weak self] in
                try? await Task.sleep(for: .milliseconds(StoreTiming.textDeltaFlushMilliseconds))
                guard let self, !Task.isCancelled else { return }
                self.flushStreamingToUI(messageId: messageId)
            }
        }
    }

    private func flushStreamingToUI(messageId: String) {
        self.textDeltaFlushTask?.cancel()
        self.textDeltaFlushTask = nil

        self.updateState { ctx in
            let updated = Self.updateMessage(messageId, messages: &ctx.messages) { message in
                message.content = self.streamingMessage
                message.isStreaming = true
            }
            if !updated {
                let origin: AssistantResponseOrigin = ctx.messages.last(where: \.belongsInConversationTrace)?.sender == .user
                    ? .userInitiated
                    : .proactive
                ctx.messages.append(
                    ChatMessage(
                        id: messageId, sender: .bobe, content: self.streamingMessage,
                        isStreaming: true,
                        responseOrigin: origin
                    )
                )
            }

            ctx.currentMessage = self.streamingMessage
            let hasVisibleText = self.hasVisibleGlyphs(self.streamingMessage)
            ctx.thinking = !hasVisibleText
            ctx.speaking = hasVisibleText
        }
        if self.hasVisibleGlyphs(self.streamingMessage) {
            self.recordLiveBobeMessage(messageId)
        }
    }

    func finalizeStreamingMessage(isComplete: Bool = true) {
        self.textDeltaFlushTask?.cancel()
        self.textDeltaFlushTask = nil

        guard let msgId = streamingMessageId else { return }

        self.updateState { ctx in
            Self.updateMessage(msgId, messages: &ctx.messages) { message in
                message.content = self.streamingMessage
                message.isStreaming = false
                message.isPending = false
                message.isComplete = isComplete
            }

            ctx.lastMessage = self.streamingMessage
            ctx.currentMessage = ""
            ctx.thinking = false
            ctx.speaking = false
        }

        self.streamingMessage = ""
        self.streamingMessageId = nil

        self.lastMessageTimer?.cancel()
        self.lastMessageTimer = Task { [weak self] in
            try? await Task.sleep(for: .seconds(StoreTiming.lastMessageClearSeconds))
            if !Task.isCancelled {
                self?.updateState { $0.lastMessage = nil }
            }
        }
    }

    private func handleConversationClosed(_ payload: ConversationClosedPayload) {
        bobeStoreLogger.info("Conversation closed: \(payload.conversationId) (\(payload.reason), \(payload.turnCount) turns)")
        self.updateState {
            $0.thinking = false
            $0.speaking = false
            $0.toolExecutions = []
            $0.conversationEnding = true
        }
        self.scheduleConversationEndSettlement()
    }

    @discardableResult
    func reloadCanonicalConversation(
        replacingActiveStream: Bool = false
    ) async -> CanonicalConversationReload {
        self.synchronizeEndpoint()
        let selection = self.selection
        let interrupted = replacingActiveStream
            ? self.freezeActiveStreamBeforeCanonicalReplacement()
            : nil
        self.conversationReloadGeneration &+= 1
        let generation = self.conversationReloadGeneration
        let mutationRevision = self.conversationMutationRevision
        let streamRevision = self.streamMutationRevision
        do {
            let snapshot = try await self.conversationSnapshot(selection)
            guard self.isCurrent(selection) else { return .unavailable }
            guard generation == self.conversationReloadGeneration,
                  mutationRevision == self.conversationMutationRevision,
                  streamRevision == self.streamMutationRevision
            else {
                return .skipped
            }
            guard replacingActiveStream
                || (self.streamingMessageId == nil && self.context.currentMessage.isEmpty)
            else {
                bobeStoreLogger.info("Conversation resync deferred during active streaming")
                return .skipped
            }
            let existingIds = Set(snapshot.turns.map(\.id))
            let pending = snapshot.pendingUpdates.filter { !existingIds.contains($0.id) }
            var messages = Self.canonicalMessages(from: (pending + snapshot.turns).sorted { $0.createdAt < $1.createdAt })
            if let interrupted,
               !messages.contains(where: { $0.id == interrupted.id }) {
                messages.append(interrupted)
            }
            self.cancelConversationClear()
            self.textDeltaFlushTask?.cancel()
            self.textDeltaFlushTask = nil
            self.streamingMessage = ""
            self.streamingMessageId = nil
            self.updateState { context in
                context.messages = Array(messages.suffix(Self.messageWindowCapacity))
                context.currentMessage = ""
                context.thinking = false
                context.speaking = false
                context.toolExecutions = []
                context.conversationEnding = false
            }
            return .applied
        } catch {
            bobeStoreLogger.error(
                "Conversation resync failed: \(error.localizedDescription, privacy: .public)"
            )
            return .unavailable
        }
    }

    @discardableResult
    func freezeActiveStreamBeforeCanonicalReplacement() -> ChatMessage? {
        guard let messageId = self.streamingMessageId else { return nil }
        self.flushStreamingToUI(messageId: messageId)
        let interrupted = self.interruptedStreamingMessage(absentFrom: [])
        self.finalizeStreamingMessage(isComplete: false)
        if interrupted == nil {
            self.updateState { context in
                Self.removeMessage(messageId, messages: &context.messages)
                context.lastMessage = nil
            }
        }
        return interrupted
    }

    static func canonicalMessages(from turns: [ConversationSnapshotTurn]) -> [ChatMessage] {
        var previousSender: MessageSender?
        return turns.compactMap { turn in
            let sender: MessageSender
            switch turn.role {
            case "user":
                sender = .user
            case "assistant":
                sender = .bobe
            default:
                return nil
            }
            let origin: AssistantResponseOrigin? = if sender == .bobe {
                previousSender == .user ? .userInitiated : .proactive
            } else {
                nil
            }
            previousSender = sender
            return ChatMessage(
                id: turn.messageId,
                sender: sender,
                content: turn.content,
                isComplete: turn.isComplete,
                responseOrigin: origin
            )
        }
    }

    private func interruptedStreamingMessage(absentFrom canonical: [ChatMessage]) -> ChatMessage? {
        guard let messageId = self.streamingMessageId,
              !canonical.contains(where: { $0.id == messageId })
        else {
            return nil
        }
        let content = self.hasVisibleGlyphs(self.streamingMessage)
            ? self.streamingMessage
            : self.context.currentMessage
        guard self.hasVisibleGlyphs(content) else { return nil }

        var message = self.context.messages.first(where: { $0.id == messageId })
            ?? ChatMessage(id: messageId, sender: .bobe, content: content)
        message.content = content
        message.isStreaming = false
        message.isPending = false
        message.isComplete = false
        return message
    }

    private func hasVisibleGlyphs(_ text: String) -> Bool {
        text.unicodeScalars.contains(where: {
            !$0.properties.isWhitespace && !CharacterSet.controlCharacters.contains($0)
        })
    }

    func scheduleConversationEndSettlement() {
        self.cancelConversationClear()
        self.conversationClearTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(StoreTiming.conversationClearSeconds))
            guard let self, !Task.isCancelled else { return }
            self.updateState { $0.conversationEnding = false }
        }
    }

    func cancelConversationClear() {
        self.conversationClearTask?.cancel()
        self.conversationClearTask = nil
    }
}
