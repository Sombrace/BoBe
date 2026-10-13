import Foundation

public struct PairingClientResult: Sendable {
    public let finalization: Data
    public let sessionKey: Data
}

public final class PairingClient: @unchecked Sendable {
    private let lock = NSLock()
    private var handle: OpaquePointer?

    deinit {
        if let handle = self.lock.withLock({
            defer { self.handle = nil }
            return self.handle
        }) {
            bobe_pairing_client_destroy(handle)
        }
    }

    public static func start(code: String) throws -> (client: PairingClient, request: Data) {
        guard code.count == 6, code.allSatisfy(\.isNumber)
        else { throw PairingClientError.invalidCode }
        let codeData = Data(code.utf8)
        var handle: OpaquePointer?
        var request = bobe_pairing_buffer(data: nil, len: 0)
        let status = codeData.withUnsafeBytes { codeBytes in
            bobe_pairing_client_start(
                codeBytes.bindMemory(to: UInt8.self).baseAddress,
                codeBytes.count,
                &handle,
                &request
            )
        }
        guard status == BOBE_PAIRING_OK, let handle else {
            throw PairingClientError.from(status)
        }
        let client = PairingClient(handle: handle)
        return (client, Self.take(&request))
    }

    public func finish(code: String, response: Data, context: Data) throws -> PairingClientResult {
        let codeData = Data(code.utf8)
        guard let handle = self.lock.withLock({
            defer { self.handle = nil }
            return self.handle
        }) else {
            throw PairingClientError.consumed
        }
        defer { bobe_pairing_client_destroy(handle) }

        var finalization = bobe_pairing_buffer(data: nil, len: 0)
        var sessionKey = bobe_pairing_buffer(data: nil, len: 0)
        let status = codeData.withUnsafeBytes { codeBytes in
            response.withUnsafeBytes { responseBytes in
                context.withUnsafeBytes { contextBytes in
                    bobe_pairing_client_finish(
                        handle,
                        codeBytes.bindMemory(to: UInt8.self).baseAddress,
                        codeBytes.count,
                        responseBytes.bindMemory(to: UInt8.self).baseAddress,
                        responseBytes.count,
                        contextBytes.bindMemory(to: UInt8.self).baseAddress,
                        contextBytes.count,
                        &finalization,
                        &sessionKey
                    )
                }
            }
        }
        guard status == BOBE_PAIRING_OK else {
            bobe_pairing_buffer_destroy(&finalization)
            bobe_pairing_buffer_destroy(&sessionKey)
            throw PairingClientError.from(status)
        }
        return PairingClientResult(
            finalization: Self.take(&finalization),
            sessionKey: Self.take(&sessionKey)
        )
    }

    private init(handle: OpaquePointer) {
        self.handle = handle
    }

    private static func take(_ buffer: inout bobe_pairing_buffer) -> Data {
        let data = buffer.data.map { Data(bytes: $0, count: buffer.len) } ?? Data()
        bobe_pairing_buffer_destroy(&buffer)
        return data
    }
}

public enum PairingClientError: LocalizedError {
    case invalidCode
    case consumed
    case authenticationFailed
    case exchangeFailed

    public var errorDescription: String? {
        switch self {
        case .invalidCode: "Enter the six-digit code shown by the BoBe server."
        case .consumed: "This pairing attempt has already finished. Start again."
        case .authenticationFailed: "The pairing code did not match or has expired."
        case .exchangeFailed: "The secure pairing exchange could not be completed."
        }
    }

    fileprivate static func from(_ status: bobe_pairing_status) -> Self {
        status == BOBE_PAIRING_AUTHENTICATION_FAILED ? .authenticationFailed : .exchangeFailed
    }
}

public enum PairingCertificate {
    public static func spkiSHA256(certificateDER: Data) -> Data? {
        var output = [UInt8](repeating: 0, count: 32)
        let status = certificateDER.withUnsafeBytes { bytes in
            bobe_pairing_spki_sha256(
                bytes.bindMemory(to: UInt8.self).baseAddress,
                bytes.count,
                &output
            )
        }
        return status == BOBE_PAIRING_OK ? Data(output) : nil
    }
}
