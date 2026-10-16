import Foundation
import Observation
import SwiftUI

@MainActor
@Observable
final class PhoneModel {
    enum Step: Int, CaseIterable {
        case welcome
        case find
        case hear
        case ready
    }

    enum Link: Equatable {
        case idle
        case connecting
        case online
        case settingUp
        case offline
        case incompatible
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
    private(set) var questionsLoad = PhoneWork.LoadState()
    private(set) var indicator: IndicatorType = .idle
    private(set) var activeTools: Set<String> = []
    private(set) var replyTrouble = false
    var draft = ""
    var tab: Tab = .bobe
    var showsSettings = false
    var pendingInvite: PairingInvite?

    let talk: HoldToTalk

    @ObservationIgnored private var paired: BrainPairing?
    @ObservationIgnored private var client: (any BrainLink)?
    @ObservationIgnored private var streamTask: Task<Void, Never>?
    @ObservationIgnored private var pollTask: Task<Void, Never>?
    @ObservationIgnored private var cursor: EventCursor?
    @ObservationIgnored private var generation = 0
    @ObservationIgnored private var answers: [UUID: (response: String, id: UUID)] = [:]
    @ObservationIgnored private lazy var messageReceipts = ReceiptBook<String> { [weak self] id in
        guard let self, let turn = ThreadMessage.presentationId(for: id) else { return .dropped }
        return await self.receipt(["conversation", turn.uuidString, "presented"])
    }
    @ObservationIgnored private lazy var questionReceipts = ReceiptBook<UUID> { [weak self] id in
        guard let self else { return .dropped }
        return await self.receipt(["owner-input", id.uuidString, "presented"])
    }
    @ObservationIgnored private(set) lazy var work = PhoneWork { [weak self] in
        self?.client
    }
    @ObservationIgnored private lazy var retirement = PairingRetirement(vault: self.vault, link: self.makeLink) { [weak self] in
        self?.paired?.credential
    }
    @ObservationIgnored private let defaults: UserDefaults
    @ObservationIgnored private let vault: BrainVault
    @ObservationIgnored private let makeLink: @MainActor (URL, BrainPairing) -> any BrainLink
    @ObservationIgnored private let exchange: @Sendable (PairingInvite) async throws -> BrainPairing

    private static let onboardedKey = "bobe.phone.onboarded"

    init(
        defaults: UserDefaults = .standard,
        vault: BrainVault = .keychain,
        talk: HoldToTalk = HoldToTalk(),
        link: @escaping @MainActor (URL, BrainPairing) -> any BrainLink = { BrainClient(origin: $0, pairing: $1) },
        exchange: @escaping @Sendable (PairingInvite) async throws -> BrainPairing = { try await PhonePairing.pair($0) }
    ) {
        self.defaults = defaults
        self.vault = vault
        self.talk = talk
        self.makeLink = link
        self.exchange = exchange
        let paired = vault.load()
        self.paired = paired
        self.brainAddress = paired?.record.origins.first?.absoluteString ?? ""
        self.isOnboarded = paired != nil && defaults.bool(forKey: Self.onboardedKey)
        self.step = paired == nil ? .welcome : .hear
    }

    var forgetPending: Bool {
        self.retirement.pending
    }

    var isConnected: Bool {
        self.link == .online || self.link == .settingUp
    }

    var needsAttention: Bool {
        [.offline, .incompatible, .removed, .settingUp].contains(self.link)
    }

    var workFailed: Bool {
        self.work.failed || self.questionsLoad.failed
    }

    var mood: BobeMood {
        switch self.link {
        case .idle, .connecting, .settingUp: .waking
        case .offline: .asleep
        case .removed, .incompatible: .worried
        case .online:
            if !self.activeTools.isEmpty {
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
        case .settingUp: L10n.tr("phone.status.setting_up")
        case .offline: L10n.tr("phone.status.offline")
        case .incompatible: L10n.tr("phone.status.update")
        case .removed: L10n.tr("phone.status.removed")
        case .online:
            if !self.activeTools.isEmpty {
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
        self.link == .online && (self.indicator != .idle || !self.activeTools.isEmpty)
    }

    var showsTyping: Bool {
        guard self.isBusy else { return false }
        guard let last = self.thread.last else { return true }
        return last.author == .me || !last.isStreaming
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
            let result = try await self.exchange(invite)
            try self.retirement.save(result, replacing: self.paired)
            self.adopt(result)
            self.retirement.resume()
            withAnimation(BobeMotion.standard) { self.pairing = .paired }
            try? await Task.sleep(for: .seconds(1.4))
            self.pairing = .idle
            if self.isOnboarded {
                self.connect()
            } else {
                self.advance(to: .hear)
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
        self.stopListening()
        if let previous = self.paired {
            self.retirement.retire(previous)
        }
        self.disconnect()
        self.vault.remove()
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

    func retryForgetting() {
        self.retirement.resume()
    }

    func start() {
        self.talk.prepare()
        self.retirement.resume()
        if self.isOnboarded { self.connect() }
    }

    func scenePhaseChanged(_ phase: ScenePhase) {
        switch phase {
        case .active:
            self.talk.prepare()
            self.retirement.resume()
            if self.isOnboarded { self.connect() }
        case .background:
            self.stopListening()
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
        let message = ThreadMessage.outgoing(content)
        withAnimation(BobeMotion.standard) { self.thread.append(message) }
        self.replyTrouble = false
        self.deliver(message.id)
    }

    func retry(_ message: ThreadMessage) {
        guard message.delivery == .failed,
              let index = self.thread.firstIndex(where: { $0.id == message.id })
        else { return }
        self.thread[index].delivery = .sending
        self.deliver(message.id)
    }

    func sendAgain(_ message: ThreadMessage) {
        guard message.delivery == .unconfirmed,
              let index = self.thread.firstIndex(where: { $0.id == message.id })
        else { return }
        self.thread[index].requestId = UUID()
        self.thread[index].delivery = .sending
        self.deliver(message.id)
    }

    func presented(_ message: ThreadMessage) {
        guard message.author == .bobe,
              message.isComplete,
              !message.isStreaming,
              message.presentationId != nil,
              !self.messageReceipts.isConfirmed(message.id)
        else { return }
        Task { await self.messageReceipts.present(message.id) }
    }

    func presented(_ question: OwnerInputWire) {
        guard !self.questionReceipts.isConfirmed(question.id) else { return }
        Task { await self.questionReceipts.present(question.id) }
    }

    func answer(_ question: OwnerInputWire, with response: String) async -> Bool {
        guard self.client != nil else { return false }
        guard await self.questionReceipts.present(question.id) == .confirmed, let client = self.client else {
            await self.refreshQuestions()
            return false
        }
        let responseId = self.answerId(for: question.id, response: response)
        do {
            let _: OwnerInputWire = try await client.send(
                ["owner-input", question.id.uuidString, "answer"],
                body: OwnerAnswer(response: response, responseId: responseId)
            )
            self.answers[question.id] = nil
            withAnimation(BobeMotion.standard) {
                self.questions.removeAll { $0.id == question.id }
            }
            return true
        } catch {
            return false
        }
    }

    func loadWork() async {
        await self.work.load()
        await self.refreshQuestions()
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
            var tick = 0
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(5))
                tick += 1
                guard let self, !Task.isCancelled else { return }
                await self.poll(refreshing: tick.isMultiple(of: 9))
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
        self.settleStream()
    }

    private func end(_ link: Link) {
        self.stopListening()
        self.disconnect()
        self.link = link
    }

    private func poll(refreshing: Bool) async {
        if self.link == .settingUp {
            await self.checkReadiness()
        }
        guard refreshing, self.isConnected else { return }
        await self.refreshQuestions()
        self.flushReceipts()
        self.work.retryFailed()
    }

    private func checkReadiness() async {
        guard let client = self.client, let paired = self.paired else { return }
        let outcome = await self.attach(client, paired)
        guard !Task.isCancelled, client === self.client else { return }
        switch outcome {
        case .attached(.ready, _):
            self.link = .online
            self.work.retryFailed()
        case .attached(.initializing, _):
            break
        case .retry:
            self.reconnect()
        case .removed:
            self.end(.removed)
        case .incompatible:
            self.end(.incompatible)
        }
    }

    private func runStream() async {
        var failures = 0
        var attempt = 0
        while !Task.isCancelled, let paired = self.paired {
            let origins = paired.record.origins
            let origin = origins[attempt % origins.count]
            self.client?.invalidate()
            let client = self.makeLink(origin, paired)
            self.client = client
            if self.link != .online, self.link != .settingUp {
                self.link = failures < origins.count ? .connecting : .offline
            }
            let outcome = await self.attach(client, paired)
            guard !Task.isCancelled else { return }
            if outcome == .removed || outcome == .incompatible {
                self.end(outcome == .removed ? .removed : .incompatible)
                return
            }
            var synced = false
            if case let .attached(readiness, _) = outcome {
                do {
                    streaming: for try await item in client.events(after: self.cursor) {
                        switch item {
                        case .connected:
                            guard await self.refreshConversation() else { break streaming }
                            synced = true
                            failures = 0
                            self.link = readiness == .ready ? .online : .settingUp
                            self.prefer(origin)
                            await self.refreshQuestions()
                            self.flushReceipts()
                            self.work.retryFailed()
                        case let .event(bundle):
                            guard await self.handle(bundle) else {
                                synced = false
                                break streaming
                            }
                        }
                    }
                } catch BrainError.unauthorized {
                    guard !Task.isCancelled else { return }
                    self.end(.removed)
                    return
                } catch {}
            }
            guard !Task.isCancelled else { return }
            self.settleStream()
            if synced {
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

    private func attach(_ client: any BrainLink, _ paired: BrainPairing) async -> PhoneAttach.Outcome {
        let outcome = await PhoneAttach.attach(
            client,
            record: paired.record,
            surface: paired.record.endpointId ?? SurfaceIdentity.current(defaults: self.defaults),
            version: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
        )
        if case let .attached(_, identity) = outcome,
           var current = self.paired,
           current.credential == paired.credential {
            let record = current.record.adopting(identity)
            if record != current.record {
                current.record = record
                self.paired = current
                try? self.vault.save(current)
            }
        }
        return outcome
    }

    private func handle(_ bundle: StreamBundle) async -> Bool {
        if let eventId = bundle.eventId,
           let cursor = self.cursor,
           bundle.runtimeIncarnationId == cursor.incarnation,
           eventId <= cursor.eventId {
            return true
        }
        switch bundle.type {
        case .textDelta:
            if let payload = try? bundle.payload.decode(as: TextDeltaPayload.self) {
                self.applyDelta(payload, messageId: bundle.messageId)
            }
        case .indicator:
            if let payload = try? bundle.payload.decode(as: IndicatorPayload.self) {
                self.indicator = payload.indicator
                if payload.indicator == .idle {
                    self.activeTools = []
                    if self.thread.contains(where: \.isStreaming) {
                        self.settleStream()
                        await self.refreshConversation()
                    }
                }
            }
        case .toolCallStart:
            if let start = try? bundle.payload.decode(as: ToolCallStartPayload.self) {
                self.activeTools.insert(start.toolCallId)
            }
        case .toolCallComplete:
            if let complete = try? bundle.payload.decode(as: ToolCallCompletePayload.self) {
                self.activeTools.remove(complete.toolCallId)
            }
        case .error:
            if let payload = try? bundle.payload.decode(as: ErrorPayload.self) {
                self.settle(payload, messageId: bundle.messageId)
            }
        case .conversationClosed:
            self.activeTools = []
            guard await self.refreshConversation() else { return false }
        case .conversationChanged:
            guard await self.refreshConversation() else { return false }
        case .snapshotRequired:
            guard await self.refreshConversation() else { return false }
            self.activeTools = []
            let gap = try? bundle.payload.decode(as: SnapshotRequiredPayload.self)
            if let resume = gap?.resumeAfter, let incarnation = bundle.runtimeIncarnationId {
                self.cursor = EventCursor(eventId: resume, incarnation: incarnation)
            } else {
                self.cursor = nil
            }
            return true
        case .bodyRequest, .unknown:
            break
        }
        if let eventId = bundle.eventId, let incarnation = bundle.runtimeIncarnationId {
            self.cursor = EventCursor(eventId: eventId, incarnation: incarnation)
        }
        return true
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
            self.thread[index].isStreaming = !payload.done
        } else {
            var message = ThreadMessage(
                id: messageId,
                author: .bobe,
                text: payload.delta,
                isComplete: payload.done,
                delivery: .sent,
                date: .now
            )
            message.isStreaming = !payload.done
            message.reachedOut = self.thread.last?.author != .me
            withAnimation(BobeMotion.standard) { self.thread.append(message) }
        }
        if payload.done {
            Task { await self.refreshQuestions() }
        }
    }

    private func settle(_ payload: ErrorPayload, messageId: String) {
        guard !payload.isTriggerError else { return }
        if let index = self.thread.lastIndex(where: { $0.id == messageId && $0.author == .bobe }) {
            self.thread[index].isStreaming = false
        }
        if payload.code != "turn_cancelled" {
            self.replyTrouble = true
        }
    }

    private func settleStream() {
        self.indicator = .idle
        self.activeTools = []
        for index in self.thread.indices where self.thread[index].isStreaming {
            self.thread[index].isStreaming = false
        }
    }

    @discardableResult
    private func refreshConversation() async -> Bool {
        guard let client = self.client else { return false }
        let generation = self.generation
        do {
            let snapshot: ConversationSnapshotResponse = try await client.get(["conversation", "current"])
            guard generation == self.generation else { return false }
            let known = Set(snapshot.turns.map(\.messageId))
            let canonical = (snapshot.turns + snapshot.pendingUpdates.filter { !known.contains($0.messageId) })
                .compactMap(ThreadMessage.init(turn:))
            self.thread = ThreadMessage.merge(canonical: canonical, local: self.thread.filter(\.isLocal))
            return true
        } catch BrainError.unauthorized {
            if generation == self.generation {
                self.end(.removed)
            }
            return false
        } catch {
            return false
        }
    }

    private func refreshQuestions() async {
        guard let client = self.client else { return }
        let generation = self.generation
        let all: [OwnerInputWire]? = try? await client.get(["owner-input"])
        guard generation == self.generation else { return }
        self.questionsLoad.record(all != nil)
        guard let all else { return }
        let pending = all.filter { $0.status == .pending }
        let pendingIds = Set(pending.map(\.id))
        self.questionReceipts.discard { !pendingIds.contains($0) }
        if pending.map(\.id) != self.questions.map(\.id) {
            withAnimation(BobeMotion.standard) { self.questions = pending }
        }
    }

    private func flushReceipts() {
        self.messageReceipts.flush()
        self.questionReceipts.flush()
    }

    private func receipt(_ path: [String]) async -> ReceiptOutcome {
        guard let client = self.client, self.isConnected else { return .retry }
        do {
            try await client.call(path)
            return .confirmed
        } catch let BrainError.http(status, _) where [400, 403, 404, 410, 422].contains(status) {
            return .dropped
        } catch {
            return .retry
        }
    }

    private func answerId(for question: UUID, response: String) -> UUID {
        if let previous = self.answers[question], previous.response == response {
            return previous.id
        }
        let id = UUID()
        self.answers[question] = (response, id)
        return id
    }

    private func deliver(_ id: String) {
        guard let index = self.thread.firstIndex(where: { $0.id == id }),
              let requestId = self.thread[index].requestId
        else { return }
        guard let client = self.client, self.link == .online else {
            self.thread[index].delivery = .failed
            return
        }
        let content = self.thread[index].text
        Task {
            var delivery = ThreadMessage.Delivery.failed
            var response: SendMessageResponse?
            do {
                response = try await client.send(
                    ["message"],
                    body: SendMessageRequest(content: content, requestId: requestId)
                )
                delivery = .sent
            } catch let BrainError.http(_, code) where code == "REQUEST_REPLAY_UNSAFE" {
                delivery = .unconfirmed
            } catch {}
            guard let index = self.thread.firstIndex(where: { $0.id == id }),
                  self.thread[index].requestId == requestId
            else { return }
            self.thread[index].delivery = delivery
            self.thread[index].replyId = response?.messageId
            if response?.replayed == true {
                await self.refreshConversation()
            }
        }
    }

    private func prefer(_ origin: URL) {
        guard var paired = self.paired, paired.record.origins.first != origin else { return }
        paired.record.origins = [origin] + paired.record.origins.filter { $0 != origin }
        self.paired = paired
        self.brainAddress = origin.absoluteString
        try? self.vault.save(paired)
    }

    private func adopt(_ result: BrainPairing) {
        self.stopListening()
        self.disconnect()
        self.paired = result
        self.brainAddress = result.record.origins.first?.absoluteString ?? ""
        self.link = .idle
        self.resetBrainState()
    }

    private func resetBrainState() {
        self.generation += 1
        self.thread = []
        self.questions = []
        self.questionsLoad = PhoneWork.LoadState()
        self.work.reset()
        self.cursor = nil
        self.messageReceipts.reset()
        self.questionReceipts.reset()
        self.answers = [:]
        self.replyTrouble = false
        self.activeTools = []
        self.indicator = .idle
    }

    private func stopListening() {
        if self.talk.state == .listening {
            self.talk.cancel()
        }
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
