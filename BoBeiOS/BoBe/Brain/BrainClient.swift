import Foundation

enum BrainError: Error, Equatable {
    case unauthorized
    case unreachable
    case http(Int, code: String?)
    case invalidResponse
}

enum BrainStreamItem: Sendable {
    case connected
    case event(StreamBundle)
}

struct EventCursor: Sendable, Equatable {
    let eventId: UInt64
    let incarnation: UUID
}

final class BrainClient: BrainLink {
    let origin: URL
    private let credential: String
    private let session: URLSession
    private let streamSession: URLSession

    init(origin: URL, pairing: BrainPairing) {
        self.origin = origin
        self.credential = pairing.credential
        let interactive = URLSessionConfiguration.ephemeral
        interactive.timeoutIntervalForRequest = 15
        interactive.timeoutIntervalForResource = 120
        interactive.waitsForConnectivity = false
        let stream = URLSessionConfiguration.ephemeral
        stream.timeoutIntervalForRequest = 75
        stream.timeoutIntervalForResource = 7 * 24 * 60 * 60
        stream.waitsForConnectivity = false
        self.session = URLSession(
            configuration: interactive,
            delegate: PairingTrustDelegate(expectedSPKI: pairing.record.pin),
            delegateQueue: nil
        )
        self.streamSession = URLSession(
            configuration: stream,
            delegate: PairingTrustDelegate(expectedSPKI: pairing.record.pin),
            delegateQueue: nil
        )
    }

    func invalidate() {
        self.session.invalidateAndCancel()
        self.streamSession.invalidateAndCancel()
    }

    func events(after cursor: EventCursor?) -> AsyncThrowingStream<BrainStreamItem, any Error> {
        var request = URLRequest(url: self.url(["events"], query: []))
        request.setValue("Bearer \(self.credential)", forHTTPHeaderField: "Authorization")
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        if let cursor {
            request.setValue(String(cursor.eventId), forHTTPHeaderField: "Last-Event-ID")
            request.setValue(cursor.incarnation.uuidString.lowercased(), forHTTPHeaderField: "BoBe-Runtime-Incarnation")
        }
        let session = self.streamSession
        let prepared = request
        return AsyncThrowingStream { continuation in
            let task = Task {
                do {
                    let (bytes, response) = try await session.bytes(for: prepared)
                    guard let http = response as? HTTPURLResponse else { throw BrainError.invalidResponse }
                    guard http.statusCode == 200 else {
                        throw http.statusCode == 401 ? BrainError.unauthorized : BrainError.http(http.statusCode, code: nil)
                    }
                    continuation.yield(.connected)
                    let decoder = JSONDecoder()
                    for try await line in bytes.lines {
                        guard line.hasPrefix("data: ") else { continue }
                        if let bundle = try? decoder.decode(StreamBundle.self, from: Data(line.dropFirst(6).utf8)) {
                            continuation.yield(.event(bundle))
                        }
                    }
                    continuation.finish()
                } catch let error as BrainError {
                    continuation.finish(throwing: error)
                } catch {
                    continuation.finish(throwing: BrainError.unreachable)
                }
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    func exchange(
        _ path: [String],
        method: String,
        body: Data?,
        query: [URLQueryItem]
    ) async throws -> Data {
        var request = URLRequest(url: self.url(path, query: query))
        request.httpMethod = method
        request.setValue("Bearer \(self.credential)", forHTTPHeaderField: "Authorization")
        if let body {
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.httpBody = body
        }
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await self.session.data(for: request)
        } catch {
            throw BrainError.unreachable
        }
        guard let http = response as? HTTPURLResponse else { throw BrainError.invalidResponse }
        guard (200 ... 299).contains(http.statusCode) else {
            if http.statusCode == 401 { throw BrainError.unauthorized }
            throw BrainError.http(http.statusCode, code: Self.code(from: data))
        }
        return data
    }

    private func url(_ path: [String], query: [URLQueryItem]) -> URL {
        var allowed = CharacterSet.urlPathAllowed
        allowed.remove(charactersIn: "/%?#")
        guard var components = URLComponents(url: self.origin, resolvingAgainstBaseURL: false) else {
            return self.origin
        }
        components.percentEncodedPath = "/" + path
            .map { $0.addingPercentEncoding(withAllowedCharacters: allowed) ?? $0 }
            .joined(separator: "/")
        components.queryItems = query.isEmpty ? nil : query
        return components.url ?? self.origin
    }

    private static func code(from data: Data) -> String? {
        struct Envelope: Decodable {
            struct Body: Decodable {
                let code: String
            }

            let error: Body
        }
        return (try? JSONDecoder().decode(Envelope.self, from: data))?.error.code
    }
}
