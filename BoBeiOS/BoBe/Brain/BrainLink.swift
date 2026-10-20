import Foundation

nonisolated protocol BrainLink: AnyObject, Sendable {
    func exchange(_ path: [String], method: String, body: Data?, query: [URLQueryItem]) async throws(BrainError) -> Data
    func events(after cursor: EventCursor?) -> AsyncThrowingStream<BrainStreamItem, any Error>
    func invalidate()
}

nonisolated extension BrainLink {
    func get<Response: Decodable & Sendable>(
        _ path: [String],
        query: [URLQueryItem] = []
    ) async throws(BrainError) -> Response {
        try Self.decode(await self.exchange(path, method: "GET", body: nil, query: query))
    }

    func send<Response: Decodable & Sendable>(
        _ path: [String],
        method: String = "POST",
        body: (any Encodable & Sendable)? = nil
    ) async throws(BrainError) -> Response {
        try Self.decode(await self.exchange(path, method: method, body: Self.encode(body), query: []))
    }

    func call(
        _ path: [String],
        method: String = "POST",
        body: (any Encodable & Sendable)? = nil
    ) async throws(BrainError) {
        _ = try await self.exchange(path, method: method, body: Self.encode(body), query: [])
    }

    private static func encode(_ body: (any Encodable & Sendable)?) throws(BrainError) -> Data? {
        guard let body else { return nil }
        do {
            return try JSONEncoder().encode(body)
        } catch {
            throw BrainError.invalidRequest
        }
    }

    private static func decode<Response: Decodable>(_ data: Data) throws(BrainError) -> Response {
        do {
            return try JSONDecoder().decode(Response.self, from: data)
        } catch {
            throw BrainError.invalidResponse
        }
    }
}
