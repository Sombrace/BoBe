import Foundation
import OSLog

nonisolated private let bodyAdapterLogger = Logger(
    subsystem: "com.bobe.app",
    category: "BodySpeechAdapter"
)

actor BodySpeechAdapter {
    static let shared = BodySpeechAdapter()

    private let stt = AppleSpeechStt(language: "en")
    private var baseURL: URL?
    private var token: String?
    private var socket: URLSessionWebSocketTask?
    private var connectionLoop: Task<Void, Never>?
    private var activeRoute: BodyRoute?
    private var captureOpen = false
    private var eouSegments: [String] = []
    private var expectedMicrophoneSequence: UInt32?
    private var contractValidated = false

    func start(configuration: BodyAdapterConfiguration) {
        guard self.connectionLoop == nil else { return }
        self.baseURL = configuration.baseURL
        self.token = configuration.bearerToken
        self.connectionLoop = Task { [weak self] in
            await self?.run()
        }
    }

    func stop() async {
        self.connectionLoop?.cancel()
        self.connectionLoop = nil
        self.socket?.cancel(with: .goingAway, reason: nil)
        self.socket = nil
        await self.clearTurn()
        self.baseURL = nil
        self.token = nil
    }

    private func run() async {
        do {
            try await self.prepareStt()
        } catch {
            bodyAdapterLogger.error(
                "Body speech models unavailable: \(error.localizedDescription, privacy: .public)"
            )
            self.connectionLoop = nil
            return
        }

        var attempt = 0
        while !Task.isCancelled {
            do {
                try await self.connectAndReceive()
                attempt = 0
            } catch is CancellationError {
                return
            } catch {
                attempt += 1
                bodyAdapterLogger.warning(
                    "Body adapter disconnected: \(error.localizedDescription, privacy: .public)"
                )
            }
            self.socket?.cancel(with: .goingAway, reason: nil)
            self.socket = nil
            await self.clearTurn()
            let delay = min(pow(2, Double(max(0, attempt - 1))) * 0.5, 15)
            do {
                try await Task.sleep(for: .seconds(delay))
            } catch {
                return
            }
        }
    }

    private func prepareStt() async throws {
        try await self.stt.load(
            onPartial: { [weak self] text in
                await self?.handlePartial(text)
            },
            onEou: { [weak self] text in
                await self?.handleEou(text)
            },
            onFailure: { _ in },
            onProgress: { _ in }
        )
    }

    private func connectAndReceive() async throws {
        guard let baseURL = self.baseURL,
              let token = self.token,
              var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false)
        else {
            throw BodySpeechAdapterError.invalidConfiguration
        }
        components.scheme = "ws"
        components.path = "/body/adapter"
        guard let url = components.url else {
            throw BodySpeechAdapterError.invalidConfiguration
        }
        var request = URLRequest(url: url)
        let endpoint = DaemonEndpoint.remote(baseURL: baseURL, bearerToken: token)
        endpoint.authorize(&request)
        request.setValue("bobe.speech-adapter.v1", forHTTPHeaderField: "Sec-WebSocket-Protocol")
        let session = DaemonTransportPolicy.session(
            for: endpoint,
            purpose: .bodyLoopbackWebSocket
        )
        let socket = session.webSocketTask(with: request)
        self.socket = socket
        self.contractValidated = false
        socket.resume()
        try await self.send(.hello)
        bodyAdapterLogger.info("Body speech adapter connected")

        while !Task.isCancelled {
            switch try await socket.receive() {
            case let .string(text):
                try await self.handleControl(text)
            case let .data(data):
                try await self.handleMedia(data)
            @unknown default:
                throw BodySpeechAdapterError.unsupportedFrame
            }
        }
    }

    private func handleControl(_ text: String) async throws {
        guard let data = text.data(using: .utf8) else {
            throw BodySpeechAdapterError.malformedControl
        }
        switch try JSONDecoder().decode(BodyAdapterServerMessage.self, from: data) {
        case let .welcome(protocolMajor, protocolMinor, adapterGeneration):
            guard protocolMajor == 1, protocolMinor == 1 else {
                throw BodySpeechAdapterError.incompatibleContract
            }
            self.contractValidated = true
            bodyAdapterLogger.info(
                "Body adapter registered generation=\(adapterGeneration, privacy: .public)"
            )
        case let .captureOpen(route, format):
            guard self.contractValidated, format == .capture else {
                throw BodySpeechAdapterError.incompatibleContract
            }
            guard self.activeRoute == nil else {
                throw BodySpeechAdapterError.routeConflict
            }
            self.captureOpen = false
            self.eouSegments.removeAll(keepingCapacity: true)
            self.expectedMicrophoneSequence = nil
            try await self.stt.reset()
            guard self.socket != nil, self.activeRoute == nil else {
                throw BodySpeechAdapterError.notConnected
            }
            self.activeRoute = route
            self.captureOpen = true
        case let .captureClose(route):
            guard route == self.activeRoute, self.captureOpen else {
                throw BodySpeechAdapterError.staleRoute
            }
            self.captureOpen = false
            try await self.finishCapture(route: route)
        case let .turnCancel(route, _):
            guard route == self.activeRoute else { return }
            await self.clearTurn()
        case let .turnComplete(route, _):
            guard route == self.activeRoute else { return }
            await self.clearTurn()
        case .unknown:
            throw BodySpeechAdapterError.malformedControl
        }
    }

    private func handleMedia(_ data: Data) async throws {
        guard self.contractValidated,
              let (header, payload) = BodyAdapterMediaHeader.parse(data),
              let route = self.activeRoute,
              header.connectionGeneration == route.connectionGeneration,
              header.leaseId == route.leaseId
        else {
            throw BodySpeechAdapterError.staleRoute
        }
        guard self.captureOpen,
              let frame = BodyLinkMicrophoneFrame.parse(payload),
              frame.streamId == route.captureStreamId,
              frame.sequence == header.sequence,
              self.expectedMicrophoneSequence == nil
                || self.expectedMicrophoneSequence == frame.sequence
                || frame.hasDiscontinuity
        else {
            throw BodySpeechAdapterError.malformedMedia
        }
        self.expectedMicrophoneSequence = frame.sequence &+ 1
        try await self.stt.acceptSamples(Self.floatSamples(from: frame.pcm))
    }

    private func handlePartial(_ text: String) async {
        guard self.captureOpen, let route = self.activeRoute else { return }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        do {
            try await self.send(.transcriptPartial(route: route, text: trimmed))
        } catch {
            bodyAdapterLogger.warning(
                "Body partial transcript failed: \(error.localizedDescription, privacy: .public)"
            )
        }
    }

    private func handleEou(_ text: String) {
        guard self.activeRoute != nil else { return }
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            self.eouSegments.append(trimmed)
        }
    }

    private func finishCapture(route: BodyRoute) async throws {
        let finished = await self.stt.finish()
            .trimmingCharacters(in: .whitespacesAndNewlines)
        let transcript = combineBodyTranscriptSegments(
            self.eouSegments,
            remainder: finished
        )
        if transcript.isEmpty {
            try await self.send(.transcriptEmpty(route: route))
        } else {
            try await self.send(.transcriptFinal(route: route, text: transcript))
        }
    }

    private func clearTurn() async {
        self.activeRoute = nil
        self.captureOpen = false
        self.eouSegments.removeAll(keepingCapacity: true)
        self.expectedMicrophoneSequence = nil
        try? await self.stt.reset()
    }

    private func send(_ message: BodyAdapterClientMessage) async throws {
        guard let socket = self.socket else {
            throw BodySpeechAdapterError.notConnected
        }
        let data = try JSONEncoder().encode(message)
        guard let text = String(data: data, encoding: .utf8) else {
            throw BodySpeechAdapterError.malformedControl
        }
        try await socket.send(.string(text))
    }

    private static func floatSamples(from pcm: Data) -> [Float] {
        let bytes = pcm.bytes
        let count = bytes.byteCount / MemoryLayout<Int16>.size
        return [Float](capacity: count) { samples in
            for index in 0 ..< count {
                let sample = bytes.load(fromByteOffset: index * MemoryLayout<Int16>.size, as: Int16.self)
                samples.append(Float(Int16(littleEndian: sample)) / 32_768)
            }
        }
    }
}

nonisolated func combineBodyTranscriptSegments(
    _ segments: [String],
    remainder: String
) -> String {
    var parts = segments
        .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        .filter { !$0.isEmpty }
    let remainder = remainder.trimmingCharacters(in: .whitespacesAndNewlines)
    if !remainder.isEmpty, parts.last != remainder {
        parts.append(remainder)
    }
    return parts.joined(separator: " ")
}

nonisolated private enum BodySpeechAdapterError: LocalizedError {
    case invalidConfiguration
    case unsupportedFrame
    case malformedControl
    case malformedMedia
    case routeConflict
    case staleRoute
    case notConnected
    case incompatibleContract

    var errorDescription: String? {
        switch self {
        case .invalidConfiguration: "Invalid body adapter configuration"
        case .unsupportedFrame: "Unsupported body adapter frame"
        case .malformedControl: "Malformed body adapter control"
        case .malformedMedia: "Malformed body adapter media"
        case .routeConflict: "A different body route is already active"
        case .staleRoute: "Body adapter route is stale"
        case .notConnected: "Body speech adapter is not connected"
        case .incompatibleContract: "Body speech adapter protocol or audio format is incompatible"
        }
    }
}
