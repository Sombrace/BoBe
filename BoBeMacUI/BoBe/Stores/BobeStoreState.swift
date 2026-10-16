import Foundation

enum BobeStateType: String, Sendable, Equatable {
    case loading
    case error
    case idle
    case thinking
    case speaking
    case wantsToSpeak = "wants_to_speak"
    case shuttingDown = "shutting_down"
}

enum MessageSender: String, Sendable {
    case user
    case bobe
}

enum AssistantResponseOrigin: Sendable, Equatable {
    case userInitiated
    case proactive
}

struct FailedSendRecovery: Identifiable, Sendable {
    let selection: EndpointSelection
    let id: String
    let content: String
    let requestId: UUID
    let canRetry: Bool
    let failureMessage: String?
}

struct ChatMessage: Identifiable, Sendable, Equatable {
    let id: String
    let sender: MessageSender
    var content: String
    var isStreaming: Bool
    var isPending: Bool
    var isComplete: Bool
    var responseOrigin: AssistantResponseOrigin?

    var hasVisibleContent: Bool {
        !self.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var belongsInConversationTrace: Bool {
        self.hasVisibleContent || self.isStreaming || self.isPending
    }

    init(
        id: String = UUID().uuidString,
        sender: MessageSender,
        content: String,
        isStreaming: Bool = false,
        isPending: Bool = false,
        isComplete: Bool = true,
        responseOrigin: AssistantResponseOrigin? = nil
    ) {
        self.id = id
        self.sender = sender
        self.content = content
        self.isStreaming = isStreaming
        self.isPending = isPending
        self.isComplete = isComplete
        self.responseOrigin = responseOrigin
    }
}

struct ToolExecution: Identifiable, Sendable {
    var id: String {
        self.toolCallId
    }

    let toolName: String
    let toolCallId: String
    var status: ToolExecutionStatus
}

enum ToolExecutionStatus: String, Sendable {
    case running
    case success
    case error
}

enum MessageComposerBlockReason: Sendable, Equatable {
    case starting
    case reconnecting
    case thinking
}

struct BobeContext: Sendable {
    var daemonConnected = false
    var daemonError = false
    var thinking = false
    var speaking = false
    var shuttingDown = false
    var lastMessage: String?
    var errorMessage: String?
    var softWarning: String?
    var acceptingUserMessages = true
    var conversationEnding = false
    var currentMessage = ""
    var messages: [ChatMessage] = []
    var failedSendRecoveries: [FailedSendRecovery] = []
    var toolExecutions: [ToolExecution] = []
    var stateType: BobeStateType = .loading
}

func deriveStateType(from context: BobeContext) -> BobeStateType {
    if context.shuttingDown { return .shuttingDown }
    if context.daemonError { return .error }
    if !context.daemonConnected { return .loading }
    if context.speaking { return .speaking }
    if context.thinking { return .thinking }
    if context.lastMessage != nil, !context.speaking { return .wantsToSpeak }
    return .idle
}
