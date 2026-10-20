@testable import BoBe
import Foundation
import Security
import SwiftUI
import Synchronization
import Testing

nonisolated enum Fixture {
    static let authority = UUID(uuidString: "6F0B3A0E-2E0C-4E0A-9A55-6E1B2D3C4F50")!
    static let otherAuthority = UUID(uuidString: "1C2D3E4F-5A6B-4C7D-8E9F-0A1B2C3D4E5F")!
    static let endpoint = UUID(uuidString: "D9C9D04A-7BEF-4747-BA22-C7D610E62612")!
    static let incarnation = UUID(uuidString: "0E8F6D2C-1B3A-4C5D-9E7F-102938475665")!
    static let phoneScopes = [
        "conversation_read", "conversation_write", "memory_read", "memory_write",
        "goals_read", "goals_write", "settings_read", "voice_session", "events_subscribe",
    ]

    static func pairing(
        _ credential: String = "bobe_v2_first",
        origin: String = "https://first.bobe.test:8766",
        authority: UUID? = Fixture.authority,
        epoch: Int? = 1
    ) -> BrainPairing {
        BrainPairing(
            record: BrainRecord(
                origins: [URL(string: origin)!],
                pin: Data(repeating: 7, count: 32),
                endpointId: authority == nil ? nil : Fixture.endpoint,
                authorityId: authority,
                authorityEpoch: epoch
            ),
            credential: credential
        )
    }

    static func hello(
        authority: UUID = Fixture.authority,
        epoch: Int = 1,
        readiness: String = "ready",
        scopes: [String] = Fixture.phoneScopes,
        negotiated: Int = 1
    ) -> Data {
        self.json([
            "runtime_authority_id": authority.uuidString.lowercased(),
            "runtime_authority_epoch": epoch,
            "daemon_version": "0.9.1",
            "negotiated_protocol": negotiated,
            "deployment_mode": "networked",
            "initialization_state": readiness == "initializing" ? "provider_required" : "ready",
            "readiness": [
                "state": readiness,
                "blocking_reasons": readiness == "ready" ? [] : ["initialization_incomplete"],
            ],
            "capabilities": ["stt": "unsupported"],
            "granted_scopes": scopes,
        ])
    }

    static func snapshot(_ turns: [[String: Any]] = []) -> Data {
        self.json(["conversation_id": NSNull(), "turns": turns, "pending_updates": []])
    }

    static func turn(_ messageId: String, role: String, content: String, complete: Bool = true) -> [String: Any] {
        [
            "id": UUID().uuidString.lowercased(),
            "message_id": messageId,
            "role": role,
            "content": content,
            "is_complete": complete,
            "created_at": "2026-10-16T00:00:00Z",
        ]
    }

    static func question(_ id: UUID, status: String = "pending") -> [String: Any] {
        [
            "id": id.uuidString.lowercased(),
            "question": "Ship it?",
            "action": "Ship the draft",
            "prompt": "Ship it?",
            "choices": [],
            "status": status,
        ]
    }

    static func goal(_ id: String, title: String = "Learn Greek") -> [String: Any] {
        [
            "id": id, "title": title, "status": "active", "priority": 1, "summary": "",
            "why_it_matters": "", "how_working_on_it": "", "patterns_observed": "",
            "attitude_feelings": "", "open_questions": "", "notes": "",
            "created_at": "2026-10-16T00:00:00Z", "updated_at": "2026-10-16T00:00:00Z",
        ]
    }

    static func accepted(_ messageId: String, replayed: Bool = false) -> Data {
        self.json(["message_id": messageId, "request_status": "queued", "replayed": replayed])
    }

    static func gap(resumeAfter: UInt64) throws -> BrainStreamItem {
        try self.event(
            "snapshot_required",
            messageId: "",
            payload: [
                "from": NSNull(),
                "to": NSNull(),
                "resume_after": resumeAfter,
                "reason": "slow_consumer",
                "snapshot_required": true,
            ]
        )
    }

    static func conversationEvent(_ type: String, eventId: UInt64) throws -> BrainStreamItem {
        let payload: [String: Any] = type == "conversation_closed"
            ? ["conversation_id": "conversation-1", "reason": "inactivity", "turn_count": 4]
            : ["conversation_id": "conversation-1"]
        return try self.event(type, messageId: "", eventId: eventId, payload: payload)
    }

    static func event(
        _ type: String,
        messageId: String = "msg_00000000000000000000000000000001",
        eventId: UInt64? = nil,
        payload: [String: Any]
    ) throws -> BrainStreamItem {
        var body: [String: Any] = [
            "type": type,
            "message_id": messageId,
            "payload": payload,
            "runtime_incarnation_id": Fixture.incarnation.uuidString.lowercased(),
        ]
        if let eventId {
            body["event_id"] = eventId
        }
        return try .event(JSONDecoder().decode(StreamBundle.self, from: self.json(body)))
    }

    static func json(_ value: Any) -> Data {
        (try? JSONSerialization.data(withJSONObject: value)) ?? Data()
    }
}

nonisolated struct Request: Sendable {
    let method: String
    let path: String
    let body: Data?

    var json: [String: Any] {
        self.body.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] } ?? [:]
    }
}

nonisolated struct StreamScript: Sendable {
    var items: [BrainStreamItem]
    var ends = false

    static let open = StreamScript(items: [.connected])
}

nonisolated final class FakeLink: BrainLink {
    private struct State {
        var replies: [String: [Result<Data, BrainError>]] = [:]
        var requests: [Request] = []
        var streams: [StreamScript] = []
        var cursors: [EventCursor?] = []
        var live: AsyncThrowingStream<BrainStreamItem, any Error>.Continuation?
    }

    private let state = Mutex(State())

    init() {
        self.reply("POST /v1/hello", Fixture.hello())
        self.reply("GET /conversation/current", Fixture.snapshot())
        self.reply("GET /owner-input", Fixture.json([]))
        self.reply("GET /goals", Fixture.json(["goals": [], "active_count": 0]))
        self.reply("GET /agent/jobs", Fixture.json([]))
        self.reply("GET /agent/schedules", Fixture.json([]))
        self.reply("GET /memory", Fixture.json(["content": "# Me\n- likes tea", "bytes": 16]))
    }

    var requests: [Request] {
        self.state.withLock { $0.requests }
    }

    var cursors: [EventCursor?] {
        self.state.withLock { $0.cursors }
    }

    func requests(_ key: String) -> [Request] {
        self.requests.filter { "\($0.method) \($0.path)" == key }
    }

    func reply(_ key: String, _ data: Data) {
        self.state.withLock { $0.replies[key] = [.success(data)] }
    }

    func fail(_ key: String, _ error: BrainError = .unreachable) {
        self.state.withLock { $0.replies[key] = [.failure(error)] }
    }

    func script(_ key: String, _ replies: [Result<Data, BrainError>]) {
        self.state.withLock { $0.replies[key] = replies }
    }

    func stream(_ script: StreamScript) {
        self.state.withLock { $0.streams.append(script) }
    }

    func emit(_ item: BrainStreamItem) {
        _ = self.state.withLock { $0.live?.yield(item) }
    }

    func exchange(
        _ path: [String],
        method: String,
        body: Data?,
        query: [URLQueryItem]
    ) async throws(BrainError) -> Data {
        let route = "/" + path.joined(separator: "/")
        let result: Result<Data, BrainError> = self.state.withLock { state in
            state.requests.append(Request(method: method, path: route, body: body))
            guard var queue = state.replies["\(method) \(route)"], let next = queue.first else { return .success(Data()) }
            if queue.count > 1 {
                queue.removeFirst()
                state.replies["\(method) \(route)"] = queue
            }
            return next
        }
        return try result.get()
    }

    func events(after cursor: EventCursor?) -> AsyncThrowingStream<BrainStreamItem, any Error> {
        let (stream, continuation) = AsyncThrowingStream<BrainStreamItem, any Error>.makeStream()
        let script = self.state.withLock { state in
            state.cursors.append(cursor)
            let script = state.streams.isEmpty ? StreamScript.open : state.streams.removeFirst()
            if !script.ends {
                state.live = continuation
            }
            return script
        }
        for item in script.items {
            continuation.yield(item)
        }
        if script.ends {
            continuation.finish()
        }
        return stream
    }

    func invalidate() {}
}

final class FakeDaemon {
    private var links: [String: FakeLink] = [:]

    func link(_ pairing: BrainPairing) -> FakeLink {
        if let link = self.links[pairing.credential] { return link }
        let link = FakeLink()
        self.links[pairing.credential] = link
        return link
    }
}

nonisolated final class MemorySecrets: SecretStore {
    private struct State {
        var items: [String: Data] = [:]
        var failingWrites: Set<String> = []
        var calls: [String] = []
    }

    private let state = Mutex(State())

    var calls: [String] {
        self.state.withLock { $0.calls }
    }

    func failWrites(to account: String) {
        self.state.withLock { _ = $0.failingWrites.insert(account) }
    }

    func read(_ account: String) -> Data? {
        self.state.withLock { $0.items[account] }
    }

    func add(_ data: Data, account: String) -> OSStatus {
        self.state.withLock { state in
            state.calls.append("add \(account)")
            if state.failingWrites.contains(account) { return errSecInteractionNotAllowed }
            if state.items[account] != nil { return errSecDuplicateItem }
            state.items[account] = data
            return errSecSuccess
        }
    }

    func update(_ data: Data, account: String) -> OSStatus {
        self.state.withLock { state in
            state.calls.append("update \(account)")
            if state.failingWrites.contains(account) { return errSecInteractionNotAllowed }
            guard state.items[account] != nil else { return errSecItemNotFound }
            state.items[account] = data
            return errSecSuccess
        }
    }

    func delete(_ account: String) {
        self.state.withLock { state in
            state.calls.append("delete \(account)")
            state.items[account] = nil
        }
    }
}

struct Harness {
    let daemon = FakeDaemon()
    let secrets = MemorySecrets()
    let defaults: UserDefaults
    let talk: HoldToTalk
    let model: PhoneModel

    var vault: BrainVault {
        BrainVault(store: self.secrets)
    }

    init(
        paired: BrainPairing? = Fixture.pairing(),
        onboarded: Bool = true,
        talk: HoldToTalk = HoldToTalk(),
        exchange: @escaping @Sendable (PairingInvite) async throws -> BrainPairing = { _ in
            throw DevicePairingError.unreachable
        }
    ) {
        let defaults = UserDefaults(suiteName: "bobe.tests.\(UUID().uuidString)") ?? .standard
        defaults.set(onboarded, forKey: "bobe.phone.onboarded")
        self.defaults = defaults
        self.talk = talk
        if let paired {
            try? BrainVault(store: self.secrets).save(paired)
        }
        let daemon = self.daemon
        self.model = PhoneModel(
            defaults: defaults,
            vault: BrainVault(store: self.secrets),
            talk: talk,
            link: { daemon.link($1) },
            exchange: exchange
        )
    }

    func link(_ pairing: BrainPairing = Fixture.pairing()) -> FakeLink {
        self.daemon.link(pairing)
    }

    func connect() async throws {
        self.model.start()
        try await eventually { self.model.link == .online }
    }

    func stop() {
        self.model.scenePhaseChanged(.background)
    }
}

func eventually(
    _ timeout: Duration = .seconds(5),
    sourceLocation: SourceLocation = #_sourceLocation,
    _ condition: @MainActor () -> Bool
) async throws {
    let deadline = ContinuousClock.now + timeout
    while !condition() {
        guard ContinuousClock.now < deadline else {
            Issue.record("Condition was not met in time", sourceLocation: sourceLocation)
            throw CancellationError()
        }
        try await Task.sleep(for: .milliseconds(10))
    }
}

func pause(_ duration: Duration = .milliseconds(200)) async {
    try? await Task.sleep(for: duration)
}
