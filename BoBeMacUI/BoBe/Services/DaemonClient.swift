import Foundation
import OSLog

private let logger = Logger(subsystem: "com.bobe.app", category: "DaemonClient")

private struct DaemonErrorEnvelope: Decodable {
    let error: DaemonErrorBody
}

private struct DaemonErrorBody: Decodable {
    let message: String
    let code: String?
}

enum RuntimeConnectionState: Sendable, Equatable {
    case connected
    case disconnected
    case repairRequired(RuntimeIssue)
}

struct EventStreamCommandOrder {
    private(set) var latest: UInt64 = 0

    mutating func accept(_ generation: UInt64) -> Bool {
        guard generation > self.latest else { return false }
        self.latest = generation
        return true
    }
}

actor DaemonClient {
    typealias Transport = @Sendable (URLRequest) async throws -> (Data, URLResponse)

    static let shared = DaemonClient()
    static let stableEventStreamUptime: Duration = .seconds(30)
    static let mcpStartupTimeout: TimeInterval = 285

    private let fixedEndpoint: DaemonEndpoint?
    var endpoint: DaemonEndpoint { self.fixedEndpoint ?? DaemonConfig.endpoint }
    var baseURL: URL { self.endpoint.baseURL }

    private let decoder: JSONDecoder
    private let encoder: JSONEncoder
    private let transport: Transport?
    let fetchTimeout: TimeInterval = 10

    private var sseTask: Task<Void, Never>?
    private var eventHandler: (@Sendable (StreamBundle) async -> Bool)?
    private var connectionHandler: (@Sendable (RuntimeConnectionState) async -> Void)?
    private var reconnectAttempts = 0
    private var isReconnecting = false
    private(set) var lastProcessedEventId: UInt64?
    private(set) var runtimeIncarnationId: UUID?
    private var sseSelection: EndpointSelection?
    private var sseCommands = EventStreamCommandOrder()

    static func shouldProcess(
        eventId: UInt64?,
        eventIncarnationId: UUID?,
        lastProcessedEventId: UInt64?,
        runtimeIncarnationId: UUID?
    ) -> Bool {
        guard let eventId,
              eventIncarnationId == runtimeIncarnationId,
              let lastProcessedEventId
        else { return true }
        return eventId > lastProcessedEventId
    }

    static func decodeEvent<Event: Decodable>(
        _ type: Event.Type,
        from line: String,
        decoder: JSONDecoder = JSONDecoder()
    ) throws -> Event? {
        guard line.hasPrefix("data: ") else { return nil }
        return try decoder.decode(type, from: Data(line.dropFirst(6).utf8))
    }

    func endpointURL(_ path: String, queryItems: [URLQueryItem] = []) -> URL {
        let segments = path.split(separator: "/").map(String.init)
        return self.endpointURL(pathSegments: segments, queryItems: queryItems)
    }

    func endpointURL(pathSegments: [String], queryItems: [URLQueryItem] = []) -> URL {
        Self.endpointURL(
            baseURL: self.baseURL,
            pathSegments: pathSegments,
            queryItems: queryItems
        )
    }

    static func endpointURL(
        baseURL: URL,
        pathSegments: [String],
        queryItems: [URLQueryItem] = []
    ) -> URL {
        guard var components = URLComponents(
            url: baseURL,
            resolvingAgainstBaseURL: false
        ) else { return baseURL }
        let prefix = components.percentEncodedPath.trimmingCharacters(
            in: CharacterSet(charactersIn: "/")
        )
        let encodedSegments = pathSegments.map(Self.encodePathSegment)
        components.percentEncodedPath = "/" + ([prefix] + encodedSegments)
            .filter { !$0.isEmpty }
            .joined(separator: "/")
        components.queryItems = queryItems.isEmpty ? nil : queryItems
        return components.url ?? baseURL
    }

    private static func encodePathSegment(_ segment: String) -> String {
        var allowed = CharacterSet.urlPathAllowed
        allowed.remove(charactersIn: "/%?#")
        return segment.addingPercentEncoding(withAllowedCharacters: allowed) ?? ""
    }

    func session(for purpose: DaemonTransportPurpose) -> URLSession {
        DaemonTransportPolicy.session(for: self.endpoint, purpose: purpose)
    }

    init(endpoint: DaemonEndpoint? = nil, transport: Transport? = nil) {
        self.fixedEndpoint = endpoint
        self.transport = transport
        self.decoder = JSONDecoder()
        self.encoder = JSONEncoder()
    }


    func connectSSE(
        generation: UInt64,
        selection: EndpointSelection,
        onEvent: @escaping @Sendable (StreamBundle) async -> Bool,
        onConnectionChange: @escaping @Sendable (RuntimeConnectionState) async -> Void
    ) {
        guard self.sseCommands.accept(generation), EndpointStore.shared.selection == selection else { return }
        if self.sseSelection != selection {
            self.sseSelection = selection
            self.lastProcessedEventId = nil
            self.runtimeIncarnationId = nil
        }
        self.eventHandler = onEvent
        self.connectionHandler = onConnectionChange
        self.isReconnecting = false
        self.reconnectAttempts = 0
        self.startSSE()
    }

    func disconnectSSE(generation: UInt64) {
        guard self.sseCommands.accept(generation) else { return }
        self.sseTask?.cancel()
        self.sseTask = nil
        self.eventHandler = nil
        self.connectionHandler = nil
    }

    private func startSSE() {
        self.sseTask?.cancel()
        self.sseTask = Task { [weak self] in
            guard let self else { return }
            await self.runSSELoop()
        }
    }

    private func runSSELoop() async {
        guard !Task.isCancelled, let selection = self.sseSelection,
              EndpointStore.shared.selection == selection
        else { return }
        let endpoint = selection.endpoint
        let url = Self.endpointURL(baseURL: endpoint.baseURL, pathSegments: ["events"])
        var request = URLRequest(url: url)
        endpoint.authorize(&request)
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        request.setValue(BodyRequestWire.offeredActions, forHTTPHeaderField: BodyRequestWire.actionsHeader)
        if let lastProcessedEventId {
            request.setValue(String(lastProcessedEventId), forHTTPHeaderField: "Last-Event-ID")
        }
        if let runtimeIncarnationId {
            request.setValue(runtimeIncarnationId.uuidString.lowercased(), forHTTPHeaderField: "BoBe-Runtime-Incarnation")
        }
        request.timeoutInterval = TimeInterval(7 * 24 * 60 * 60)

        var connectedAt: ContinuousClock.Instant?
        do {
            let session = DaemonTransportPolicy.session(for: endpoint, purpose: .persistentEventStream)
            let (bytes, response) = try await session.bytes(for: request)
            guard !Task.isCancelled else { return }
            guard let httpResponse = response as? HTTPURLResponse else {
                logger.warning("SSE connection failed without an HTTP response")
                await self.handleSSEDisconnect(connectedFor: nil)
                return
            }
            guard httpResponse.statusCode == 200 else {
                let statusCode = httpResponse.statusCode
                logger.warning("SSE connection failed with HTTP \(statusCode)")
                if statusCode == 401 {
                    await self.handleTerminalSSEFailure(AttachError.unauthorized)
                } else {
                    await self.handleSSEDisconnect(connectedFor: nil)
                }
                return
            }

            logger.info("SSE connected")
            connectedAt = ContinuousClock.now
            await self.connectionHandler?(.connected)
            if let handler = self.eventHandler {
                try await self.receiveEvents(bytes.lines, handler: handler)
            }
        } catch {
            if !Task.isCancelled {
                logger.warning("SSE stream error: \(error.localizedDescription, privacy: .public)")
                if RuntimeIssue.isSecureVerificationFailure(error as NSError) {
                    await self.handleTerminalSSEFailure(error)
                    return
                }
            }
        }

        if !Task.isCancelled {
            await self.handleSSEDisconnect(connectedFor: connectedAt.map { ContinuousClock.now - $0 })
        }
    }

    func receiveEvents<Lines: AsyncSequence>(
        _ lines: Lines,
        handler: @Sendable (StreamBundle) async -> Bool
    ) async throws where Lines.Element == String {
        for try await line in lines {
            guard !Task.isCancelled else { return }
            let bundle: StreamBundle
            do {
                guard let decoded = try Self.decodeEvent(StreamBundle.self, from: line, decoder: self.decoder) else {
                    continue
                }
                bundle = decoded
            } catch {
                logger.error(
                    "Undecodable SSE event; reconnecting without advancing the cursor: \(error.localizedDescription, privacy: .public)"
                )
                return
            }
            guard Self.shouldProcess(
                eventId: bundle.eventId,
                eventIncarnationId: bundle.runtimeIncarnationId,
                lastProcessedEventId: self.lastProcessedEventId,
                runtimeIncarnationId: self.runtimeIncarnationId
            ) else {
                continue
            }
            let processed = await handler(bundle)
            guard !Task.isCancelled else { return }
            guard processed else {
                logger.error("SSE event recovery failed; reconnecting without advancing the cursor")
                return
            }
            if self.acknowledge(bundle) {
                self.reconnectAttempts = 0
            }
        }
    }

    private func acknowledge(_ bundle: StreamBundle) -> Bool {
        let previous = (self.lastProcessedEventId, self.runtimeIncarnationId)
        if bundle.type == .snapshotRequired,
           let gap = try? bundle.payload.decode(as: SnapshotRequiredPayload.self) {
            self.runtimeIncarnationId = bundle.runtimeIncarnationId
            self.lastProcessedEventId = gap.resumeAfter
        } else if let eventId = bundle.eventId,
                  let incarnationId = bundle.runtimeIncarnationId {
            self.runtimeIncarnationId = incarnationId
            self.lastProcessedEventId = eventId
        }
        return self.lastProcessedEventId != nil
            && (self.lastProcessedEventId, self.runtimeIncarnationId) != previous
    }

    func reconnectDelay(connectedFor uptime: Duration?) -> Duration {
        if let uptime, uptime >= Self.stableEventStreamUptime {
            self.reconnectAttempts = 0
        }
        self.reconnectAttempts += 1
        guard self.reconnectAttempts > 1 else { return .zero }
        return .seconds(min(pow(2.0, Double(self.reconnectAttempts - 2)) * 0.5, 15.0))
    }

    private func handleTerminalSSEFailure(_ error: any Error) async {
        guard !Task.isCancelled else { return }
        self.sseTask?.cancel()
        self.sseTask = nil
        self.isReconnecting = false
        await self.connectionHandler?(.repairRequired(RuntimeIssue(error)))
    }

    private func handleSSEDisconnect(connectedFor uptime: Duration?) async {
        guard !Task.isCancelled, !self.isReconnecting else { return }
        let generation = self.sseCommands.latest
        self.isReconnecting = true
        defer { if generation == self.sseCommands.latest { self.isReconnecting = false } }

        await self.connectionHandler?(.disconnected)
        guard !Task.isCancelled, generation == self.sseCommands.latest else { return }
        let delay = self.reconnectDelay(connectedFor: uptime)
        if delay > .zero {
            logger.info("SSE reconnecting in \(delay, privacy: .public) (attempt \(self.reconnectAttempts))")
            try? await Task.sleep(for: delay)
        }
        if !Task.isCancelled, generation == self.sseCommands.latest {
            self.startSSE()
        }
    }


    private func send(
        url: URL,
        routeLabel: String,
        method: String,
        body: (any Encodable)?,
        requestTimeout: TimeInterval?,
        acceptedStatusCodes: Set<Int> = []
    ) async throws -> Data {
        var request = URLRequest(url: url)
        self.endpoint.authorize(&request)
        request.httpMethod = method
        request.timeoutInterval = requestTimeout ?? self.fetchTimeout

        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = try self.encoder.encode(AnyEncodable(body))
        }

        let data: Data
        let response: URLResponse
        do {
            if let transport = self.transport {
                (data, response) = try await transport(request)
            } else {
                (data, response) = try await self.session(for: .interactiveHTTP).data(for: request)
            }
        } catch {
            let description = error.localizedDescription
            logger.error(
                "\(method, privacy: .public) \(routeLabel, privacy: .public): network error — \(description, privacy: .public)"
            )
            throw error
        }
        guard let httpResponse = response as? HTTPURLResponse else {
            logger.error("\(method, privacy: .public) \(routeLabel, privacy: .public): invalid response (not HTTP)")
            throw DaemonError.invalidResponse
        }
        guard (200 ... 299).contains(httpResponse.statusCode)
            || acceptedStatusCodes.contains(httpResponse.statusCode)
        else {
            let (message, code) = self.errorMessage(from: data)
            let codeLabel = code ?? "no-code"
            let statusCode = httpResponse.statusCode
            logger.error(
                // swiftlint:disable:next line_length
                "\(method, privacy: .public) \(routeLabel, privacy: .public) failed: HTTP \(statusCode) — \(message, privacy: .public) [\(codeLabel, privacy: .public)]"
            )
            throw DaemonError.httpError(statusCode: statusCode, message: message, code: code)
        }
        return data
    }

    private func send(
        _ path: String,
        method: String,
        body: (any Encodable)?,
        queryItems: [URLQueryItem],
        requestTimeout: TimeInterval?,
        acceptedStatusCodes: Set<Int> = []
    ) async throws -> Data {
        try await self.send(
            url: self.endpointURL(path, queryItems: queryItems),
            routeLabel: path,
            method: method,
            body: body,
            requestTimeout: requestTimeout,
            acceptedStatusCodes: acceptedStatusCodes
        )
    }

    func fetch<T: Decodable>(
        _ path: String,
        method: String = "GET",
        body: (any Encodable)? = nil,
        queryItems: [URLQueryItem] = [],
        requestTimeout: TimeInterval? = nil
    ) async throws -> T {
        let data = try await self.send(
            path,
            method: method,
            body: body,
            queryItems: queryItems,
            requestTimeout: requestTimeout
        )
        do {
            return try self.decoder.decode(T.self, from: data)
        } catch {
            logger.error("\(method, privacy: .public) \(path, privacy: .public): decode error — \(error.localizedDescription, privacy: .public)")
            throw error
        }
    }

    func fetch<T: Decodable>(
        pathSegments: [String],
        method: String = "GET",
        body: (any Encodable)? = nil,
        requestTimeout: TimeInterval? = nil
    ) async throws -> T {
        let label = "/" + pathSegments.joined(separator: "/")
        let data = try await self.send(
            url: self.endpointURL(pathSegments: pathSegments),
            routeLabel: label,
            method: method,
            body: body,
            requestTimeout: requestTimeout
        )
        do {
            return try self.decoder.decode(T.self, from: data)
        } catch {
            logger.error(
                "\(method, privacy: .public) \(label, privacy: .public): decode error — \(error.localizedDescription, privacy: .public)"
            )
            throw error
        }
    }

    func fetchVoid(
        _ path: String,
        method: String = "POST",
        body: (any Encodable)? = nil,
        queryItems: [URLQueryItem] = [],
        requestTimeout: TimeInterval? = nil
    ) async throws {
        _ = try await self.send(
            path,
            method: method,
            body: body,
            queryItems: queryItems,
            requestTimeout: requestTimeout
        )
    }

    func fetchVoid(
        pathSegments: [String],
        method: String = "POST",
        body: (any Encodable)? = nil,
        requestTimeout: TimeInterval? = nil
    ) async throws {
        let label = "/" + pathSegments.joined(separator: "/")
        _ = try await self.send(
            url: self.endpointURL(pathSegments: pathSegments),
            routeLabel: label,
            method: method,
            body: body,
            requestTimeout: requestTimeout
        )
    }

    private func errorMessage(from data: Data) -> (message: String, code: String?) {
        if let envelope = try? self.decoder.decode(DaemonErrorEnvelope.self, from: data) {
            return (envelope.error.message, envelope.error.code)
        }
        return (String(data: data, encoding: .utf8) ?? "Unknown error", nil)
    }


    func health() async throws -> HealthResponse {
        try await self.fetch("/health")
    }

    func readiness() async throws -> ReadinessResponse {
        let data = try await self.send(
            "/readyz",
            method: "GET",
            body: nil,
            queryItems: [],
            requestTimeout: nil,
            acceptedStatusCodes: [503]
        )
        return try self.decoder.decode(ReadinessResponse.self, from: data)
    }

    @discardableResult
    func advanceInitialization(to state: InitializationStateWire) async throws -> InitializationResponse {
        try await self.fetch(
            "/initialization",
            method: "POST",
            body: InitializationAdvanceRequest(initializationState: state),
            requestTimeout: 95
        )
    }


    func completeBodyRequest(_ requestId: String, reply: BodyReply) async throws {
        try await self.fetchVoid(pathSegments: ["body", "requests", requestId], body: reply)
    }


    @discardableResult
    func sendMessage(_ content: String, requestId: UUID) async throws -> SendMessageResponse {
        try await self.fetch(
            "/message",
            method: "POST",
            body: SendMessageRequest(content: content, requestId: requestId)
        )
    }


    func getMemory() async throws -> MemoryResponse {
        try await self.fetch("/memory")
    }

    @discardableResult
    func updateMemory(_ content: String) async throws -> MemoryResponse {
        try await self.fetch(
            "/memory",
            method: "PUT",
            body: MemoryUpdateRequest(content: content)
        )
    }


    func listGoals(status: GoalStatus? = nil, includeArchived: Bool = false) async throws -> GoalListResponse {
        var queryItems: [URLQueryItem] = []
        if let status, status != .unknown {
            queryItems.append(URLQueryItem(name: "status", value: status.rawValue))
        }
        if includeArchived {
            queryItems.append(URLQueryItem(name: "include_archived", value: "true"))
        }
        return try await self.fetch("/goals", queryItems: queryItems)
    }

    func createGoal(_ request: GoalCreateRequest) async throws -> Goal {
        try await self.fetch("/goals", method: "POST", body: request)
    }

    func updateGoal(_ id: String, _ request: GoalUpdateRequest) async throws -> Goal {
        try await self.fetch(pathSegments: ["goals", id], method: "PATCH", body: request)
    }

    func deleteGoal(_ id: String) async throws {
        try await self.fetchVoid(pathSegments: ["goals", id], method: "DELETE")
    }

    func completeGoal(_ id: String) async throws {
        try await self.fetchVoid(pathSegments: ["goals", id, "complete"], method: "POST")
    }

    func archiveGoal(_ id: String) async throws {
        try await self.fetchVoid(pathSegments: ["goals", id, "archive"], method: "POST")
    }


    func listSouls() async throws -> SoulListResponse {
        try await self.fetch("/souls")
    }

    func createSoul(_ request: SoulCreateRequest) async throws -> Soul {
        try await self.fetch("/souls", method: "POST", body: request)
    }

    func updateSoul(_ id: String, _ request: SoulUpdateRequest) async throws -> Soul {
        try await self.fetch(pathSegments: ["souls", id], method: "PATCH", body: request)
    }

    func deleteSoul(_ id: String) async throws {
        try await self.fetchVoid(pathSegments: ["souls", id], method: "DELETE")
    }

    func enableSoul(_ id: String) async throws {
        try await self.fetchVoid(pathSegments: ["souls", id, "enable"], method: "POST")
    }

    func disableSoul(_ id: String) async throws {
        try await self.fetchVoid(pathSegments: ["souls", id, "disable"], method: "POST")
    }


    func listUserProfiles() async throws -> UserProfileListResponse {
        try await self.fetch("/user-profiles")
    }

    func createUserProfile(_ request: UserProfileCreateRequest) async throws -> UserProfile {
        try await self.fetch("/user-profiles", method: "POST", body: request)
    }

    func updateUserProfile(_ id: String, _ request: UserProfileUpdateRequest) async throws -> UserProfile {
        try await self.fetch(pathSegments: ["user-profiles", id], method: "PATCH", body: request)
    }

    func deleteUserProfile(_ id: String) async throws {
        try await self.fetchVoid(pathSegments: ["user-profiles", id], method: "DELETE")
    }

    func enableUserProfile(_ id: String) async throws {
        try await self.fetchVoid(pathSegments: ["user-profiles", id, "enable"], method: "POST")
    }

    func disableUserProfile(_ id: String) async throws {
        try await self.fetchVoid(pathSegments: ["user-profiles", id, "disable"], method: "POST")
    }
}
