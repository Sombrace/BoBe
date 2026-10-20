import Foundation

public struct PairingClientResult: Sendable {
    public let finalization: Data
    public let sessionKey: Data
}

public struct PairingClient: ~Copyable {
    private let handle: OpaquePointer
    public let request: Data

    public init(code: String) throws(PairingClientError) {
        guard code.count == 6, code.allSatisfy(\.isNumber)
        else { throw .invalidCode }
        let codeData = Data(code.utf8)
        var handle: OpaquePointer?
        var request = bobe_pairing_buffer(data: nil, len: 0)
        let status = codeData.span.withUnsafeBufferPointer { code in
            bobe_pairing_client_start(code.baseAddress, code.count, &handle, &request)
        }
        guard status == BOBE_PAIRING_OK, let handle else {
            throw .from(status)
        }
        self.handle = handle
        self.request = Self.take(&request)
    }

    deinit {
        bobe_pairing_client_destroy(self.handle)
    }

    public consuming func finish(
        code: String,
        response: Data,
        context: Data
    ) throws(PairingClientError) -> PairingClientResult {
        let codeData = Data(code.utf8)
        var finalization = bobe_pairing_buffer(data: nil, len: 0)
        var sessionKey = bobe_pairing_buffer(data: nil, len: 0)
        let status = codeData.span.withUnsafeBufferPointer { code in
            response.span.withUnsafeBufferPointer { response in
                context.span.withUnsafeBufferPointer { context in
                    bobe_pairing_client_finish(
                        self.handle,
                        code.baseAddress,
                        code.count,
                        response.baseAddress,
                        response.count,
                        context.baseAddress,
                        context.count,
                        &finalization,
                        &sessionKey
                    )
                }
            }
        }
        guard status == BOBE_PAIRING_OK else {
            bobe_pairing_buffer_destroy(&finalization)
            bobe_pairing_buffer_destroy(&sessionKey)
            throw .from(status)
        }
        return PairingClientResult(
            finalization: Self.take(&finalization),
            sessionKey: Self.take(&sessionKey)
        )
    }

    private static func take(_ buffer: inout bobe_pairing_buffer) -> Data {
        let data = buffer.data.map { Data(bytes: $0, count: buffer.len) } ?? Data()
        bobe_pairing_buffer_destroy(&buffer)
        return data
    }
}

public enum PairingClientError: LocalizedError {
    case invalidCode
    case authenticationFailed
    case exchangeFailed

    public var errorDescription: String? {
        switch self {
        case .invalidCode: "Enter the six-digit code shown by the BoBe server."
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
        let status = certificateDER.span.withUnsafeBufferPointer { der in
            bobe_pairing_spki_sha256(der.baseAddress, der.count, &output)
        }
        return status == BOBE_PAIRING_OK ? Data(output) : nil
    }
}
