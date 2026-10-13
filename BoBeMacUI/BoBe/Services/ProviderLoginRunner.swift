import AppKit
import Foundation
import Observation

protocol ProviderLoginClient: Sendable {
    func startProviderLogin(profile: String) async throws
    func cancelProviderLogin(profile: String) async throws
    func providerLoginStatus(profile: String) async throws -> ProviderLoginPhase
    func submitProviderLogin(profile: String, value: String) async throws
    func streamProviderLogin(
        profile: String,
        onPhase: @Sendable @escaping (ProviderLoginPhase) async -> Void
    ) async throws
}

extension DaemonClient: ProviderLoginClient {}

@MainActor
@Observable
final class ProviderLoginRunner {
    enum State: Equatable {
        case idle
        case preparing
        case awaitingUser(url: URL, code: String)
        case polling(url: URL, code: String)
        case awaitingInput(message: String, password: Bool)
        case completed
        case failed(message: String)
        case canceled

        var isTerminal: Bool {
            switch self {
            case .completed, .failed, .canceled: true
            case .idle, .preparing, .awaitingUser, .polling, .awaitingInput: false
            }
        }
    }

    private(set) var state: State = .idle
    private(set) var didCopyCode = false
    private(set) var isSubmitting = false
    private(set) var isCanceling = false
    private(set) var submissionError: String?
    private var streamTask: Task<Void, Never>?
    private var generation: UInt64 = 0
    private var requiresCancellation = false
    private let profile: String
    private let selection: EndpointSelection
    private let client: any ProviderLoginClient
    private let currentSelection: @MainActor () -> EndpointSelection

    init(
        profile: String,
        selection: EndpointSelection,
        client: (any ProviderLoginClient)? = nil,
        currentSelection: @escaping @MainActor () -> EndpointSelection = { EndpointStore.shared.selection }
    ) {
        self.profile = profile
        self.selection = selection
        self.client = client ?? DaemonClient(endpoint: selection.endpoint)
        self.currentSelection = currentSelection
    }

    func start() {
        guard self.streamTask == nil, !self.isCanceling, self.currentSelection() == self.selection else { return }
        self.generation &+= 1
        let generation = self.generation
        self.state = .preparing
        self.didCopyCode = false
        self.submissionError = nil
        self.streamTask = Task { [weak self] in
            guard let self else { return }
            defer { if self.generation == generation { self.streamTask = nil } }
            do {
                if self.requiresCancellation {
                    try await self.client.cancelProviderLogin(profile: self.profile)
                }
                try Task.checkCancellation()
                guard self.isCurrent(generation) else { return }
                self.requiresCancellation = true
                try await self.client.startProviderLogin(profile: self.profile)
                for attempt in 0 ..< 5 {
                    try Task.checkCancellation()
                    guard self.isCurrent(generation) else { return }
                    do {
                        try await self.client.streamProviderLogin(profile: self.profile) { [weak self] phase in
                            await self?.apply(phase, generation: generation)
                        }
                        if self.state.isTerminal { return }
                    } catch {
                        try Task.checkCancellation()
                        guard self.isCurrent(generation) else { return }
                        if attempt == 4 { throw error }
                    }
                    let phase = try await self.client.providerLoginStatus(profile: self.profile)
                    self.apply(phase, generation: generation)
                    if self.state.isTerminal { return }
                    try await Task.sleep(for: .seconds(min(attempt + 1, 5)))
                }
                guard self.isCurrent(generation) else { return }
                self.state = .failed(message: L10n.tr("setup.cloud_auth.error.stream_ended"))
            } catch is CancellationError {
                return
            } catch {
                guard self.isCurrent(generation) else { return }
                self.state = .failed(message: error.localizedDescription)
            }
        }
    }

    @discardableResult
    func cancel() async -> Bool {
        guard !self.isCanceling else { return false }
        self.generation &+= 1
        self.streamTask?.cancel()
        self.streamTask = nil
        guard self.requiresCancellation else {
            if self.state != .completed { self.state = .canceled }
            return true
        }
        self.isCanceling = true
        defer { self.isCanceling = false }
        do {
            try await self.client.cancelProviderLogin(profile: self.profile)
            self.requiresCancellation = false
            self.state = .canceled
            return true
        } catch {
            self.state = .failed(message: error.localizedDescription)
            return false
        }
    }

    func submit(_ value: String) async {
        guard case .awaitingInput = self.state, !self.isSubmitting,
              self.currentSelection() == self.selection
        else { return }
        let generation = self.generation
        let submittedState = self.state
        self.isSubmitting = true
        self.submissionError = nil
        defer { self.isSubmitting = false }
        do {
            try await self.client.submitProviderLogin(profile: self.profile, value: value)
            guard self.isCurrent(generation) else { return }
            if self.state == submittedState { self.state = .preparing }
        } catch {
            guard self.isCurrent(generation) else { return }
            self.submissionError = error.localizedDescription
        }
    }

    func openVerificationURL() {
        guard self.currentSelection() == self.selection else { return }
        switch self.state {
        case let .awaitingUser(url, _), let .polling(url, _):
            NSWorkspace.shared.open(url)
        default:
            break
        }
    }

    func copyCode() {
        let code: String
        switch self.state {
        case let .awaitingUser(_, value), let .polling(_, value):
            code = value
        default:
            return
        }
        guard !code.isEmpty else { return }
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(code, forType: .string)
        self.didCopyCode = true
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(2))
            self?.didCopyCode = false
        }
    }

    private func isCurrent(_ generation: UInt64) -> Bool {
        self.generation == generation && self.currentSelection() == self.selection
    }

    private func apply(_ phase: ProviderLoginPhase, generation: UInt64) {
        guard self.isCurrent(generation) else { return }
        switch phase {
        case .preparing:
            self.state = .preparing
        case let .awaitingUser(url, code), let .polling(url, code):
            guard let parsed = URL(string: url), parsed.host != nil,
                  ["https", "http"].contains(parsed.scheme?.lowercased() ?? "")
            else {
                self.state = .failed(message: L10n.tr("setup.cloud_auth.error.invalid_url"))
                self.streamTask?.cancel()
                return
            }
            if case .polling = phase {
                self.state = .polling(url: parsed, code: code)
            } else {
                self.state = .awaitingUser(url: parsed, code: code)
            }
        case let .awaitingInput(message, password):
            self.submissionError = nil
            self.state = .awaitingInput(message: message, password: password)
        case .completed:
            self.requiresCancellation = false
            self.state = .completed
        case let .failed(message):
            self.requiresCancellation = false
            self.state = .failed(message: message)
        case .canceled:
            self.requiresCancellation = false
            self.state = .canceled
        case let .unsupported(phase):
            self.state = .failed(message: L10n.tr("setup.cloud_auth.error.unsupported_phase_format", phase))
        }
    }
}
