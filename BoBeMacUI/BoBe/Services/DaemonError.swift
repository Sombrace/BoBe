import Foundation

nonisolated enum DaemonError: Error, LocalizedError {
    case invalidResponse
    case httpError(statusCode: Int, message: String, code: String? = nil)

    var errorDescription: String? {
        switch self {
        case .invalidResponse: "Invalid response from daemon"
        case let .httpError(code, msg, _): "HTTP \(code): \(msg)"
        }
    }
}

nonisolated struct AnyEncodable: Encodable {
    private let encode: (Encoder) throws -> Void

    init(_ wrapped: any Encodable) {
        self.encode = wrapped.encode
    }

    func encode(to encoder: Encoder) throws {
        try self.encode(encoder)
    }
}
