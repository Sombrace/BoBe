import CryptoKit
import Foundation
import PairingClient

nonisolated enum PairingTrust: Sendable {
    case loopback
    case firstUse
    case pinned(Data)
}

nonisolated struct PairingTarget: Sendable {
    let url: URL
    let transport: String
    let trust: PairingTrust

    var origin: String {
        self.url.absoluteString
    }
}

nonisolated struct PairingEnrollment: Encodable, Sendable {
    let endpointId: String
    let label: String
    let protocolMin: Int
    let protocolMax: Int
    let capabilities: [String: Bool]

    enum CodingKeys: String, CodingKey {
        case endpointId = "endpoint_id"
        case label
        case protocolMin = "protocol_min"
        case protocolMax = "protocol_max"
        case capabilities
    }

    func renamed(_ endpointId: String) -> Self {
        Self(
            endpointId: endpointId,
            label: self.label,
            protocolMin: self.protocolMin,
            protocolMax: self.protocolMax,
            capabilities: self.capabilities
        )
    }
}

nonisolated struct PairingOutcome: Sendable {
    let endpointId: UUID
    let credential: String
    let runtimeAuthorityId: UUID
    let runtimeAuthorityEpoch: Int
    let tlsSPKISha256: Data?
}

nonisolated enum PairingExchange {
    static let protocolName = "bobe-pairing-opaque-ristretto255-v1"

    private struct Connection {
        let target: PairingTarget
        let session: URLSession
        let observedSPKI: Data?
    }

    private struct ExpectedContext {
        let origin: String
        let transport: String
        let spki: String
        let scopeProfile: String
        let authorityId: UUID
        let authorityEpoch: Int
        let clientNonce: Data
        let serverNonce: Data
    }

    private struct BeginRequest: Encodable {
        let origin: String
        let tlsSpkiSha256: String
        let request: String
        let clientNonce: String

        enum CodingKeys: String, CodingKey {
            case origin
            case tlsSpkiSha256 = "tls_spki_sha256"
            case request
            case clientNonce = "client_nonce"
        }
    }

    private struct BeginResponse: Decodable {
        let exchangeId: UUID
        let response: String
        let serverNonce: String
        let runtimeAuthorityId: UUID
        let runtimeAuthorityEpoch: Int
        let tlsSpkiSha256: String
        let context: String
        let `protocol`: String

        enum CodingKeys: String, CodingKey {
            case exchangeId = "exchange_id"
            case response
            case serverNonce = "server_nonce"
            case runtimeAuthorityId = "runtime_authority_id"
            case runtimeAuthorityEpoch = "runtime_authority_epoch"
            case tlsSpkiSha256 = "tls_spki_sha256"
            case context
            case `protocol`
        }
    }

    private struct CompleteRequest: Encodable {
        let exchangeId: UUID
        let origin: String
        let tlsSpkiSha256: String
        let clientNonce: String
        let serverNonce: String
        let finalization: String
        let nonce: String
        let enrollment: String

        enum CodingKeys: String, CodingKey {
            case exchangeId = "exchange_id"
            case origin
            case tlsSpkiSha256 = "tls_spki_sha256"
            case clientNonce = "client_nonce"
            case serverNonce = "server_nonce"
            case finalization, nonce, enrollment
        }
    }

    private struct CompleteResponse: Decodable {
        let nonce: String
        let enrollment: String
    }

    private struct EnrollmentResponse: Decodable {
        let endpointId: String
        let credential: String
        let runtimeAuthorityId: UUID
        let runtimeAuthorityEpoch: Int

        enum CodingKeys: String, CodingKey {
            case endpointId = "endpoint_id"
            case credential
            case runtimeAuthorityId = "runtime_authority_id"
            case runtimeAuthorityEpoch = "runtime_authority_epoch"
        }
    }

    @concurrent
    static func run(
        _ target: PairingTarget,
        code: String,
        scopeProfile: String,
        enrollment: PairingEnrollment
    ) async throws -> PairingOutcome {
        let (session, observedSPKI) = try await self.openSession(for: target)
        defer { session.invalidateAndCancel() }
        let connection = Connection(target: target, session: session, observedSPKI: observedSPKI)
        do {
            return try await self.exchange(over: connection, code: code, scopeProfile: scopeProfile, enrollment: enrollment)
        } catch DevicePairingError.retired {
            return try await self.exchange(
                over: connection,
                code: code,
                scopeProfile: scopeProfile,
                enrollment: enrollment.renamed(SurfaceIdentity.renew().uuidString)
            )
        }
    }

    private static func exchange(
        over connection: Connection,
        code: String,
        scopeProfile: String,
        enrollment: PairingEnrollment
    ) async throws -> PairingOutcome {
        let target = connection.target
        let session = connection.session
        let spki = connection.observedSPKI?.hexEncodedString() ?? ""
        let clientNonce = try self.randomData(count: 32)
        let client = try PairingClient(code: code)
        let begin: BeginResponse = try await self.post(
            BeginRequest(
                origin: target.origin,
                tlsSpkiSha256: spki,
                request: client.request.base64URLEncodedString(),
                clientNonce: clientNonce.base64URLEncodedString()
            ),
            to: target.url.appendingPathComponent("v2/pairing/begin"),
            session: session
        )
        guard begin.protocol == self.protocolName,
              begin.tlsSpkiSha256 == spki,
              let context = Data(base64URLEncoded: begin.context),
              let serverNonce = Data(base64URLEncoded: begin.serverNonce),
              self.verifyContext(
                context,
                expected: ExpectedContext(
                    origin: target.origin,
                    transport: target.transport,
                    spki: spki,
                    scopeProfile: scopeProfile,
                    authorityId: begin.runtimeAuthorityId,
                    authorityEpoch: begin.runtimeAuthorityEpoch,
                    clientNonce: clientNonce,
                    serverNonce: serverNonce
                )
              ),
              let response = Data(base64URLEncoded: begin.response)
        else { throw DevicePairingError.identityMismatch }

        let finished = try client.finish(code: code, response: response, context: context)
        let key = SymmetricKey(
            data: HKDF<SHA256>.deriveKey(
                inputKeyMaterial: SymmetricKey(data: finished.sessionKey),
                info: Data("bobe.pairing.enrollment.aead.v1".utf8),
                outputByteCount: 32
            ).withUnsafeBytes { Data($0) }
        )
        let sealed = try ChaChaPoly.seal(JSONEncoder().encode(enrollment), using: key, authenticating: context)
        let complete: CompleteResponse = try await self.post(
            CompleteRequest(
                exchangeId: begin.exchangeId,
                origin: target.origin,
                tlsSpkiSha256: spki,
                clientNonce: clientNonce.base64URLEncodedString(),
                serverNonce: begin.serverNonce,
                finalization: finished.finalization.base64URLEncodedString(),
                nonce: sealed.nonce.data.base64URLEncodedString(),
                enrollment: (sealed.ciphertext + sealed.tag).base64URLEncodedString()
            ),
            to: target.url.appendingPathComponent("v2/pairing/complete"),
            session: session
        )
        guard let responseNonce = Data(base64URLEncoded: complete.nonce),
              let encrypted = Data(base64URLEncoded: complete.enrollment)
        else { throw DevicePairingError.invalidResponse }
        let box = try ChaChaPoly.SealedBox(
            nonce: .init(data: responseNonce),
            ciphertext: encrypted.dropLast(16),
            tag: encrypted.suffix(16)
        )
        let result = try JSONDecoder().decode(
            EnrollmentResponse.self,
            from: ChaChaPoly.open(box, using: key, authenticating: context)
        )
        guard result.runtimeAuthorityId == begin.runtimeAuthorityId,
              result.runtimeAuthorityEpoch == begin.runtimeAuthorityEpoch,
              let endpointId = UUID(uuidString: result.endpointId)
        else { throw DevicePairingError.identityMismatch }
        return PairingOutcome(
            endpointId: endpointId,
            credential: result.credential,
            runtimeAuthorityId: result.runtimeAuthorityId,
            runtimeAuthorityEpoch: result.runtimeAuthorityEpoch,
            tlsSPKISha256: connection.observedSPKI
        )
    }

    private static func openSession(for target: PairingTarget) async throws -> (URLSession, Data?) {
        switch target.trust {
        case .loopback:
            return (URLSession(configuration: .ephemeral, delegate: PairingTrustDelegate(), delegateQueue: nil), nil)
        case .firstUse:
            let delegate = PairingTrustDelegate(acceptsUntrustedPairingCertificate: true)
            let session = URLSession(configuration: .ephemeral, delegate: delegate, delegateQueue: nil)
            do {
                return (session, try await self.observePin(target, session: session, delegate: delegate))
            } catch {
                session.invalidateAndCancel()
                throw error
            }
        case let .pinned(expected):
            let delegate = PairingTrustDelegate(acceptsUntrustedPairingCertificate: true)
            let probe = URLSession(configuration: .ephemeral, delegate: delegate, delegateQueue: nil)
            defer { probe.invalidateAndCancel() }
            guard try await self.observePin(target, session: probe, delegate: delegate) == expected else {
                throw DevicePairingError.identityMismatch
            }
            let pinned = PairingTrustDelegate(expectedSPKI: expected)
            return (URLSession(configuration: .ephemeral, delegate: pinned, delegateQueue: nil), expected)
        }
    }

    private static func observePin(
        _ target: PairingTarget,
        session: URLSession,
        delegate: PairingTrustDelegate
    ) async throws -> Data {
        var probe = URLRequest(url: target.url.appendingPathComponent("health"))
        probe.timeoutInterval = 10
        do {
            _ = try await session.data(for: probe)
        } catch {
            throw DevicePairingError.unreachable
        }
        guard let pin = delegate.observedPin() else { throw DevicePairingError.unreachable }
        return pin
    }

    private static func verifyContext(_ data: Data, expected: ExpectedContext) -> Bool {
        guard let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return false
        }
        return value["protocol"] as? String == self.protocolName
            && value["origin"] as? String == expected.origin
            && value["transport"] as? String == expected.transport
            && value["tls_spki_sha256"] as? String == expected.spki
            && value["authority_id"] as? String == expected.authorityId.uuidString.lowercased()
            && value["authority_epoch"] as? Int == expected.authorityEpoch
            && value["client_nonce"] as? String == expected.clientNonce.base64URLEncodedString()
            && value["server_nonce"] as? String == expected.serverNonce.base64URLEncodedString()
            && value["scope_profile"] as? String == expected.scopeProfile
    }

    private static func post<Request: Encodable, Response: Decodable>(
        _ body: Request,
        to url: URL,
        session: URLSession
    ) async throws -> Response {
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.timeoutInterval = 15
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(body)
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw DevicePairingError.rejected }
        if http.statusCode == 409 { throw DevicePairingError.retired }
        guard http.statusCode == 200 else { throw DevicePairingError.rejected }
        return try JSONDecoder().decode(Response.self, from: data)
    }

    private static func randomData(count: Int) throws -> Data {
        var data = Data(count: count)
        let status = data.withUnsafeMutableBytes { bytes in
            guard let baseAddress = bytes.baseAddress else { return errSecParam }
            return SecRandomCopyBytes(kSecRandomDefault, bytes.count, baseAddress)
        }
        guard status == errSecSuccess else { throw DevicePairingError.randomness }
        return data
    }
}

nonisolated enum DevicePairingError: LocalizedError {
    case invalidAddress, unreachable, identityMismatch, invalidResponse, rejected, retired, randomness

    var errorDescription: String? {
        switch self {
        case .invalidAddress: L10n.tr("setup.runtime.error.pair_invalid_address")
        case .unreachable: L10n.tr("setup.runtime.error.pair_unreachable")
        case .identityMismatch: L10n.tr("setup.runtime.error.pair_identity_mismatch")
        case .rejected, .retired: L10n.tr("setup.runtime.error.pair_rejected")
        case .invalidResponse: L10n.tr("setup.runtime.error.pair_invalid_response")
        case .randomness: L10n.tr("setup.runtime.error.pair_randomness")
        }
    }
}

nonisolated extension Data {
    func base64URLEncodedString() -> String {
        self.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    init?(base64URLEncoded value: String) {
        var value = value.replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        value += String(repeating: "=", count: (4 - value.count % 4) % 4)
        self.init(base64Encoded: value)
    }

    func hexEncodedString() -> String {
        self.map { String(format: "%02x", $0) }.joined()
    }
}

nonisolated private extension ChaChaPoly.Nonce {
    var data: Data {
        self.withUnsafeBytes { Data($0) }
    }
}
