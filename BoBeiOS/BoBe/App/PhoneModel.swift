import Foundation
import Observation
import SwiftUI

@MainActor
@Observable
final class PhoneModel {
    enum Step: Int, CaseIterable {
        case welcome
        case find
        case reach
        case hear
        case ready
    }

    enum Link: Equatable {
        case idle
        case connecting
        case online
        case offline
        case removed
    }

    enum Pairing: Equatable {
        case idle
        case working
        case paired
        case failed(String)
    }

    enum Tab: Hashable {
        case bobe
        case work
        case memory
    }

    private(set) var isOnboarded: Bool
    private(set) var step: Step
    private(set) var link: Link = .idle
    private(set) var pairing: Pairing = .idle
    private(set) var brainAddress: String
    private(set) var thread: [ThreadMessage] = []
    private(set) var questions: [OwnerInputWire] = []
    private(set) var goals: [Goal] = []
    private(set) var jobs: [AgentJobWire] = []
    private(set) var schedules: [AgentScheduleWire] = []
    private(set) var workLoaded = false
    private(set) var memory: String?
    private(set) var indicator: IndicatorType = .idle
    private(set) var activeTool: String?
    private(set) var replyTrouble = false
    var draft = ""
    var tab: Tab = .bobe
    var showsSettings = false
    var pendingInvite: PairingInvite?

    let talk = HoldToTalk()

    @ObservationIgnored private var paired: BrainPairing?
    @ObservationIgnored private var client: BrainClient?
    @ObservationIgnored private var streamTask: Task<Void, Never>?
    @ObservationIgnored private var pollTask: Task<Void, Never>?
    @ObservationIgnored private var cursor: EventCursor?
    @ObservationIgnored private var presentedQuestions: Set<UUID> = []
    @ObservationIgnored private var presentedMessages: Set<String> = []
    @ObservationIgnored private let defaults: UserDefaults

    private static let onboardedKey = "bobe.phone.onboarded"

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        let paired = BrainVault.load()
        self.paired = paired
        self.brainAddress = paired?.record.origins.first?.absoluteString ?? ""
        self.isOnboarded = paired != nil && defaults.bool(forKey: Self.onboardedKey)
        self.step = paired == nil ? .welcome : .reach
    }

    var mood: BobeMood {
        switch self.link {
        case .idle, .connecting: .waking
        case .offline: .asleep
        case .removed: .worried
        case .online:
            if self.activeTool != nil {
                .working
            } else if self.indicator == .streaming {
                .speaking
            } else if self.indicator == .thinking {
                .thinking
            } else if !self.questions.isEmpty {
                .eager
            } else {
                .idle
            }
        }
    }

    var status: String {
        switch self.link {
        case .idle, .connecting: L10n.tr("phone.status.connecting")
        case .offline: L10n.tr("phone.status.offline")
        case .removed: L10n.tr("phone.status.removed")
        case .online:
            if self.activeTool != nil {
                L10n.tr("phone.status.working")
            } else if self.indicator == .streaming {
                L10n.tr("phone.status.replying")
            } else if self.indicator == .thinking {
                L10n.tr("phone.status.thinking")
            } else if !self.questions.isEmpty {
                L10n.tr("phone.status.question")
            } else {
                L10n.tr("phone.status.here")
            }
        }
    }

    var isBusy: Bool {
        self.link == .online && (self.indicator != .idle || self.activeTool != nil)
    }

    var showsTyping: Bool {
        guard self.isBusy else { return false }
        guard let last = self.thread.last else { return true }
        return last.author == .me || last.isComplete
    }

    func begin() {
        self.advance(to: .find)
    }

    func returnToWelcome() {
        self.clearPairingError()
        self.advance(to: .welcome)
    }

    func open(_ url: URL) {
        guard let invite = PairingInvite(link: url) else { return }
        if self.paired != nil, self.isOnboarded, self.link != .removed {
            self.pendingInvite = invite
            return
        }
        if !self.isOnboarded {
            self.advance(to: .find)
        }
        Task { await self.pair(invite) }
    }

    func acceptPendingInvite() {
        guard let invite = self.pendingInvite else { return }
        self.pendingInvite = nil
        Task { await self.pair(invite) }
    }

    func pair(_ invite: PairingInvite) async {
        guard self.pairing != .working else { return }
        withAnimation(BobeMotion.standard) { self.pairing = .working }
        do {
            let result = try await PhonePairing.pair(invite)
            try BrainVault.save(result)
            self.adopt(result)
            withAnimation(BobeMotion.standard) { self.pairing = .paired }
            try? await Task.sleep(for: .seconds(1.4))
            self.pairing = .idle
            if self.isOnboarded {
                self.connect()
            } else {
                self.advance(to: .reach)
            }
        } catch {
            withAnimation(BobeMotion.standard) { self.pairing = .failed(Self.describe(error)) }
        }
    }

    func clearPairingError() {
        if case .failed = self.pairing {
            self.pairing = .idle
        }
    }

    func allowNotifications() async {
        _ = await PhonePermissions.requestNotifications()
        self.advance(to: .hear)
    }

    func skipNotifications() {
        self.advance(to: .hear)
    }

    func allowVoice() async {
        _ = await PhonePermissions.requestVoice()
        self.talk.prepare()
        self.advance(to: .ready)
    }

    func skipVoice() {
        self.advance(to: .ready)
    }

    func finishOnboarding() {
        self.defaults.set(true, forKey: Self.onboardedKey)
        withAnimation(BobeMotion.soft) { self.isOnboarded = true }
        self.connect()
    }

    func unpair() {
        if let paired = self.paired, let origin = paired.record.origins.first {
            let farewell = BrainClient(origin: origin, pairing: paired)
            Task {
                try? await farewell.call(["endpoints", "self"], method: "DELETE")
                farewell.invalidate()
            }
        }
        self.disconnect()
        BrainVault.remove()
        self.defaults.removeObject(forKey: Self.onboardedKey)
        self.defaults.removeObject(forKey: SurfaceIdentity.defaultsKey)
        self.paired = nil
        self.resetBrainState()
        self.brainAddress = ""
        self.link = .idle
        self.tab = .bobe
        self.showsSettings = false
        self.step = .welcome
        withAnimation(BobeMotion.soft) { self.isOnboarded = false }
    }

    func start() {
        self.talk.prepare()
        if self.isOnboarded { self.connect() }
    }

    func scenePhaseChanged(_ phase: ScenePhase) {
        switch phase {
        case .active:
            self.talk.prepare()
            if self.isOnboarded { self.connect() }
        case .background:
            self.disconnect()
        default:
            break
        }
    }

    func reconnect() {
        self.disconnect()
        self.connect()
    }

    func send(_ text: String? = nil) {
        let content = (text ?? self.draft).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !content.isEmpty else { return }
        if text == nil { self.draft = "" }
        let requestId = UUID()
        let message = ThreadMessage(
            id: "local-" + requestId.uuidString,
            author: .me,
            text: content,
            isComplete: true,
            delivery: .sending,
            date: .now
        )
        withAnimation(BobeMotion.standard) { self.thread.append(message) }
        self.replyTrouble = false
        self.deliver(message, requestId: requestId)
    }

    func retry(_ message: ThreadMessage) {
        guard message.delivery == .failed,
              let index = self.thread.firstIndex(where: { $0.id == message.id })
        else { return }
        self.thread[index].delivery = .sending
        self.deliver(self.thread[index], requestId: UUID())
    }

    func presented(_ message: ThreadMessage) {
        guard message.isComplete,
              !self.presentedMessages.contains(message.id),
              let id = message.presentationId,
              let client = self.client
        else { return }
        self.presentedMessages.insert(message.id)
        Task {
            try? await client.call(["conversation", id.uuidString, "presented"])
        }
    }

    func presented(_ question: OwnerInputWire) {
        guard !self.presentedQuestions.contains(question.id), let client = self.client else { return }
        self.presentedQuestions.insert(question.id)
        Task {
            try? await client.call(["owner-input", question.id.uuidString, "presented"])
        }
    }

    func answer(_ question: OwnerInputWire, with response: String) async -> Bool {
        guard let client = self.client else { return false }
        do {
            let _: OwnerInputWire = try await client.send(
                ["owner-input", question.id.uuidString, "answer"],
                body: OwnerAnswer(response: response, responseId: UUID())
            )
            withAnimation(BobeMotion.standard) {
                self.questions.removeAll { $0.id == question.id }
            }
            return true
        } catch {
            return false
        }
    }

    func loadWork() async {
        guard let client = self.client else { return }
        async let goals: GoalListResponse? = try? client.get(["goals"])
        async let jobs: [AgentJobWire]? = try? client.get(["agent", "jobs"])
        async let schedules: [AgentScheduleWire]? = try? client.get(["agent", "schedules"])
        let (loadedGoals, loadedJobs, loadedSchedules) = await (goals, jobs, schedules)
        if let loadedGoals { self.goals = loadedGoals.goals }
        if let loadedJobs { self.jobs = loadedJobs }
        if let loadedSchedules { self.schedules = loadedSchedules }
        self.workLoaded = true
        await self.refreshQuestions()
    }

    func loadMemory() async {
        guard let client = self.client else { return }
        if let response: MemoryResponse = try? await client.get(["memory"]) {
            self.memory = response.content
        }
    }

    func setGoal(_ goal: Goal, paused: Bool) async {
        guard let client = self.client else { return }
        if let updated: Goal = try? await client.send(
            ["goals", goal.id],
            method: "PATCH",
            body: GoalUpdateRequest(status: paused ? .paused : .active)
        ), let index = self.goals.firstIndex(where: { $0.id == goal.id }) {
            withAnimation(BobeMotion.standard) { self.goals[index] = updated }
        }
    }

    func completeGoal(_ goal: Goal) async {
        guard let client = self.client else { return }
        do {
            try await client.call(["goals", goal.id, "complete"])
            await self.loadWork()
        } catch {}
    }

    func createGoal(title: String, why: String) async -> Bool {
        guard let client = self.client else { return false }
        let reason = why.trimmingCharacters(in: .whitespacesAndNewlines)
        do {
            let goal: Goal = try await client.send(
                ["goals"],
                body: GoalCreateRequest(
                    title: title.trimmingCharacters(in: .whitespacesAndNewlines),
                    whyItMatters: reason.isEmpty ? nil : reason
                )
            )
            withAnimation(BobeMotion.standard) { self.goals.insert(goal, at: 0) }
            return true
        } catch {
            return false
        }
    }

    func askToForget() {
        self.draft = L10n.tr("phone.memory.forget.prefill")
        self.tab = .bobe
    }

    private func connect() {
        guard self.paired != nil, self.streamTask == nil else { return }
        self.streamTask = Task { [weak self] in
            await self?.runStream()
        }
        self.pollTask?.cancel()
        self.pollTask = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(45))
                guard let self, self.link == .online else { continue }
                await self.refreshQuestions()
            }
        }
    }

    private func disconnect() {
        self.streamTask?.cancel()
        self.streamTask = nil
        self.pollTask?.cancel()
        self.pollTask = nil
        self.client?.invalidate()
        self.client = nil
        if self.link != .removed {
            self.link = .idle
        }
        self.indicator = .idle
        self.activeTool = nil
    }

    private func runStream() async {
        var failures = 0
        var attempt = 0
        while !Task.isCancelled, let paired = self.paired {
            let origins = paired.record.origins
            let origin = origins[attempt % origins.count]
            self.client?.invalidate()
            let client = BrainClient(origin: origin, pairing: paired)
            self.client = client
            if self.link != .online {
                self.link = failures < origins.count ? .connecting : .offline
            }
            var connected = false
            do {
                for try await item in client.events(after: self.cursor) {
                    switch item {
                    case .connected:
                        connected = true
                        failures = 0
                        self.link = .online
                        self.prefer(origin)
                        await self.refreshConversation()
                        await self.refreshQuestions()
                    case let .event(bundle):
                        await self.handle(bundle)
                    }
                }
            } catch BrainError.unauthorized {
                self.link = .removed
                self.streamTask = nil
                return
            } catch {}
            guard !Task.isCancelled else { return }
            self.indicator = .idle
            self.activeTool = nil
            if connected {
                try? await Task.sleep(for: .seconds(0.5))
                continue
            }
            failures += 1
            attempt += 1
            if failures >= origins.count {
                self.link = .offline
            }
            let delay = failures < origins.count ? 0.3 : min(0.5 * pow(2, Double(min(failures, 6))), 20)
            try? await Task.sleep(for: .seconds(delay))
        }
    }

    private func handle(_ bundle: StreamBundle) async {
        if let eventId = bundle.eventId,
           let cursor = self.cursor,
           bundle.runtimeIncarnationId == cursor.incarnation,
           eventId <= cursor.eventId {
            return
        }
        switch bundle.type {
        case .textDelta:
            if let payload = try? bundle.payload.decode(as: TextDeltaPayload.self) {
                self.applyDelta(payload, messageId: bundle.messageId)
            }
        case .indicator:
            if let payload = try? bundle.payload.decode(as: IndicatorPayload.self) {
                self.indicator = payload.indicator
                if payload.indicator == .idle { self.activeTool = nil }
            }
        case .toolCallStart:
            self.activeTool = (try? bundle.payload.decode(as: ToolCallStartPayload.self))?.toolName
        case .toolCallComplete:
            self.activeTool = nil
        case .error:
            self.replyTrouble = true
        case .conversationChanged, .conversationClosed:
            await self.refreshConversation()
        case .snapshotRequired:
            let gap = try? bundle.payload.decode(as: SnapshotRequiredPayload.self)
            if let resume = gap?.resumeAfter, let incarnation = bundle.runtimeIncarnationId {
                self.cursor = EventCursor(eventId: resume, incarnation: incarnation)
            } else {
                self.cursor = nil
            }
            await self.refreshConversation()
            return
        case .bodyRequest, .unknown:
            break
        }
        if let eventId = bundle.eventId, let incarnation = bundle.runtimeIncarnationId {
            self.cursor = EventCursor(eventId: eventId, incarnation: incarnation)
        }
    }

    private func applyDelta(_ payload: TextDeltaPayload, messageId: String) {
        self.replyTrouble = false
        if let index = self.thread.lastIndex(where: { $0.id == messageId && $0.author == .bobe }) {
            if payload.done {
                self.thread[index].text = payload.delta
            } else {
                self.thread[index].text += payload.delta
            }
            self.thread[index].isComplete = payload.done
        } else {
            var message = ThreadMessage(
                id: messageId,
                author: .bobe,
                text: payload.delta,
                isComplete: payload.done,
                delivery: .sent,
                date: .now
            )
            message.reachedOut = self.thread.last?.author != .me
            withAnimation(BobeMotion.standard) { self.thread.append(message) }
        }
        if payload.done {
            Task { await self.refreshQuestions() }
        }
    }

    private func refreshConversation() async {
        guard let client = self.client else { return }
        do {
            let snapshot: ConversationSnapshotResponse = try await client.get(["conversation", "current"])
            let known = Set(snapshot.turns.map(\.messageId))
            let canonical = (snapshot.turns + snapshot.pendingUpdates.filter { !known.contains($0.messageId) })
                .compactMap(ThreadMessage.init(turn:))
            let canonicalIds = Set(canonical.map(\.id))
            let unsent = self.thread.filter { $0.delivery != .sent && !canonicalIds.contains($0.id) }
            self.thread = ThreadMessage.ordered(canonical) + unsent
        } catch BrainError.unauthorized {
            self.link = .removed
        } catch {}
    }

    private func refreshQuestions() async {
        guard let client = self.client else { return }
        if let all: [OwnerInputWire] = try? await client.get(["owner-input"]) {
            let pending = all.filter { $0.status == .pending }
            if pending.map(\.id) != self.questions.map(\.id) {
                withAnimation(BobeMotion.standard) { self.questions = pending }
            }
        }
    }

    private func deliver(_ message: ThreadMessage, requestId: UUID) {
        guard let client = self.client, self.link == .online else {
            self.markFailed(message.id)
            return
        }
        Task {
            do {
                let response: SendMessageResponse = try await client.send(
                    ["message"],
                    body: SendMessageRequest(content: message.text, requestId: requestId)
                )
                guard let index = self.thread.firstIndex(where: { $0.id == message.id }) else { return }
                if response.replayed {
                    self.thread.remove(at: index)
                    await self.refreshConversation()
                } else {
                    self.thread[index].delivery = .sent
                }
            } catch {
                self.markFailed(message.id)
            }
        }
    }

    private func markFailed(_ id: String) {
        if let index = self.thread.firstIndex(where: { $0.id == id }) {
            self.thread[index].delivery = .failed
        }
    }

    private func prefer(_ origin: URL) {
        guard var paired = self.paired, paired.record.origins.first != origin else { return }
        paired.record.origins = [origin] + paired.record.origins.filter { $0 != origin }
        self.paired = paired
        self.brainAddress = origin.absoluteString
        try? BrainVault.save(paired)
    }

    private func adopt(_ result: BrainPairing) {
        self.disconnect()
        self.paired = result
        self.brainAddress = result.record.origins.first?.absoluteString ?? ""
        self.resetBrainState()
    }

    private func resetBrainState() {
        self.thread = []
        self.questions = []
        self.goals = []
        self.jobs = []
        self.schedules = []
        self.workLoaded = false
        self.memory = nil
        self.cursor = nil
        self.presentedQuestions = []
        self.presentedMessages = []
        self.replyTrouble = false
    }

    private func advance(to step: Step) {
        withAnimation(BobeMotion.standard) { self.step = step }
    }

    private static func describe(_ error: any Error) -> String {
        if let error = error as? DevicePairingError, let description = error.errorDescription {
            return description
        }
        if error is BrainVaultError {
            return L10n.tr("phone.pair.error.save")
        }
        return L10n.tr("phone.pair.error.generic")
    }
}

private struct OwnerAnswer: Encodable, Sendable {
    let response: String
    let responseId: UUID

    enum CodingKeys: String, CodingKey {
        case response
        case responseId = "response_id"
    }
}
