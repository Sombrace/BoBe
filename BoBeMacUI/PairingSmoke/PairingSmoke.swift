import CryptoKit
import Darwin
import Foundation
import PairingClient
import Security

@main
struct PairingSmoke {
    static func main() async throws {
        guard CommandLine.arguments.count == 3 else {
            throw SmokeError.usage
        }
        let address = CommandLine.arguments[1]
        let code = CommandLine.arguments[2]
        let result = try await SmokePairer().pair(address: address, code: code)
        if let output = ProcessInfo.processInfo.environment["BOBE_PAIRING_CREDENTIAL_OUTPUT"] {
            try result.credential.write(toFile: output, atomically: true, encoding: .utf8)
            try FileManager.default.setAttributes(
                [.posixPermissions: 0o600],
                ofItemAtPath: output
            )
        }
        print(String(data: try JSONEncoder().encode(result.report), encoding: .utf8) ?? "{}")
    }
}

private struct SmokeOutcome {
    let report: SmokeResult
    let credential: String
}

private struct SmokeResult: Encodable {
    let authorityId: UUID
    let authorityEpoch: Int
    let endpointId: String
    let spkiSha256: String
    let credentialPrefix: String
}

private struct BeginRequest: Encodable {
    let origin: String
    let tls_spki_sha256: String
    let request: String
    let client_nonce: String
}

private struct BeginResponse: Decodable {
    let exchange_id: UUID
    let response: String
    let server_nonce: String
    let runtime_authority_id: UUID
    let runtime_authority_epoch: Int
    let tls_spki_sha256: String
    let context: String
    let `protocol`: String
}

private struct CompleteRequest: Encodable {
    let exchange_id: UUID
    let origin: String
    let tls_spki_sha256: String
    let client_nonce: String
    let server_nonce: String
    let finalization: String
    let nonce: String
    let enrollment: String
}

private struct CompleteResponse: Decodable {
    let nonce: String
    let enrollment: String
}

private struct EnrollmentRequest: Encodable {
    let endpoint_id: String
    let label: String
    let protocol_min: Int
    let protocol_max: Int
    let capabilities: [String: Bool]
}

private struct EnrollmentResponse: Decodable {
    let endpoint_id: String
    let credential: String
    let runtime_authority_id: UUID
    let runtime_authority_epoch: Int
}

private final class SmokePairer {
    func pair(address: String, code: String) async throws -> SmokeOutcome {
        let pairing = try canonicalOrigin(address)
        let delegate = SmokeTrustDelegate()
        let session = URLSession(configuration: .ephemeral, delegate: delegate, delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        let pinHex: String
        if pairing.transport == "https" {
            _ = try? await session.data(from: pairing.url.appendingPathComponent("health"))
            guard let pin = delegate.pin else { throw SmokeError.noCertificate }
            pinHex = pin.map { String(format: "%02x", $0) }.joined()
        } else {
            pinHex = ""
        }
        let origin = pairing.url
        let clientNonce = try randomData(count: 32)
        let started = try PairingClient.start(code: code)
        let begin: BeginResponse = try await post(
            BeginRequest(
                origin: origin.absoluteString,
                tls_spki_sha256: pinHex,
                request: base64url(started.request),
                client_nonce: base64url(clientNonce)
            ),
            origin.appendingPathComponent("v2/pairing/begin"),
            session
        )
        guard begin.protocol == "bobe-pairing-opaque-ristretto255-v1",
              begin.tls_spki_sha256 == pinHex,
              let context = fromBase64url(begin.context),
              let serverNonce = fromBase64url(begin.server_nonce),
              contextMatches(
                context,
                origin: origin.absoluteString,
                transport: pairing.transport,
                spki: pinHex,
                authorityId: begin.runtime_authority_id,
                authorityEpoch: begin.runtime_authority_epoch,
                clientNonce: clientNonce,
                serverNonce: serverNonce
              ),
              let response = fromBase64url(begin.response)
        else { throw SmokeError.invalidResponse }
        let finished = try started.client.finish(code: code, response: response, context: context)
        let key = HKDF<SHA256>.deriveKey(
            inputKeyMaterial: SymmetricKey(data: finished.sessionKey),
            info: Data("bobe.pairing.enrollment.aead.v1".utf8),
            outputByteCount: 32
        )
        let endpointId = UUID().uuidString
        let plaintext = try JSONEncoder().encode(
            EnrollmentRequest(
                endpoint_id: endpointId,
                label: "Pairing Smoke",
                protocol_min: 1,
                protocol_max: 1,
                capabilities: ["microphone": true]
            )
        )
        let sealed = try ChaChaPoly.seal(plaintext, using: key, authenticating: context)
        let complete: CompleteResponse = try await post(
            CompleteRequest(
                exchange_id: begin.exchange_id,
                origin: origin.absoluteString,
                tls_spki_sha256: pinHex,
                client_nonce: base64url(clientNonce),
                server_nonce: begin.server_nonce,
                finalization: base64url(finished.finalization),
                nonce: base64url(sealed.nonce.withUnsafeBytes { Data($0) }),
                enrollment: base64url(sealed.ciphertext + sealed.tag)
            ),
            origin.appendingPathComponent("v2/pairing/complete"),
            session
        )
        guard let nonce = fromBase64url(complete.nonce),
              let ciphertextAndTag = fromBase64url(complete.enrollment)
        else { throw SmokeError.invalidResponse }
        let box = try ChaChaPoly.SealedBox(
            nonce: .init(data: nonce),
            ciphertext: ciphertextAndTag.dropLast(16),
            tag: ciphertextAndTag.suffix(16)
        )
        let enrolled = try JSONDecoder().decode(
            EnrollmentResponse.self,
            from: ChaChaPoly.open(box, using: key, authenticating: context)
        )
        var authenticated = URLRequest(url: origin.appendingPathComponent("endpoints"))
        authenticated.setValue(
            "Bearer \(enrolled.credential)",
            forHTTPHeaderField: "Authorization"
        )
        let (_, authenticatedResponse) = try await session.data(for: authenticated)
        guard (authenticatedResponse as? HTTPURLResponse)?.statusCode == 200 else {
            throw SmokeError.rejected
        }
        return SmokeOutcome(
            report: SmokeResult(
                authorityId: enrolled.runtime_authority_id,
                authorityEpoch: enrolled.runtime_authority_epoch,
                endpointId: enrolled.endpoint_id,
                spkiSha256: pinHex,
                credentialPrefix: String(enrolled.credential.prefix(8))
            ),
            credential: enrolled.credential
        )
    }

    private func post<R: Encodable, S: Decodable>(
        _ body: R,
        _ url: URL,
        _ session: URLSession
    ) async throws -> S {
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(body)
        let (data, response) = try await session.data(for: request)
        guard (response as? HTTPURLResponse)?.statusCode == 200 else {
            throw SmokeError.rejected
        }
        return try JSONDecoder().decode(S.self, from: data)
    }
}

private final class SmokeTrustDelegate: NSObject, URLSessionDelegate, @unchecked Sendable {
    private(set) var pin: Data?

    func urlSession(
        _ session: URLSession,
        didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping @Sendable (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        guard let trust = challenge.protectionSpace.serverTrust,
              let chain = SecTrustCopyCertificateChain(trust) as? [SecCertificate],
              let leaf = chain.first
        else {
            completionHandler(.cancelAuthenticationChallenge, nil)
            return
        }
        let der = SecCertificateCopyData(leaf) as Data
        guard let pin = PairingCertificate.spkiSHA256(certificateDER: der) else {
            completionHandler(.cancelAuthenticationChallenge, nil)
            return
        }
        self.pin = pin
        completionHandler(.useCredential, URLCredential(trust: trust))
    }
}

private struct SmokePairingOrigin {
    let url: URL
    let transport: String
}

private func canonicalOrigin(_ address: String) throws -> SmokePairingOrigin {
    let trimmed = address.trimmingCharacters(in: .whitespacesAndNewlines)
    let candidate = trimmed.contains("://") ? trimmed : "https://\(trimmed)"
    guard var components = URLComponents(string: candidate),
          let scheme = components.scheme?.lowercased(),
          let host = components.host?.lowercased(),
          !host.isEmpty,
          components.user == nil,
          components.password == nil,
          components.query == nil,
          components.fragment == nil,
          components.path.isEmpty || components.path == "/"
    else { throw SmokeError.usage }
    let transport: String
    switch scheme {
    case "https": transport = "https"
    case "http" where isLoopbackHost(host): transport = "loopback_http"
    default: throw SmokeError.usage
    }
    components.scheme = scheme
    components.host = host
    components.path = ""
    if (scheme == "https" && components.port == 443)
        || (scheme == "http" && components.port == 80) {
        components.port = nil
    }
    guard let url = components.url else { throw SmokeError.usage }
    return SmokePairingOrigin(url: url, transport: transport)
}

private func isLoopbackHost(_ host: String) -> Bool {
    let normalized = host.lowercased()
    if normalized == "localhost" || normalized == "::1" { return true }
    var address = in_addr()
    guard normalized.withCString({ inet_pton(AF_INET, $0, &address) }) == 1 else {
        return false
    }
    return withUnsafeBytes(of: &address) { $0.first == 127 }
}

private func contextMatches(
    _ data: Data,
    origin: String,
    transport: String,
    spki: String,
    authorityId: UUID,
    authorityEpoch: Int,
    clientNonce: Data,
    serverNonce: Data
) -> Bool {
    guard let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
        return false
    }
    return value["protocol"] as? String == "bobe-pairing-opaque-ristretto255-v1"
        && value["origin"] as? String == origin
        && value["transport"] as? String == transport
        && value["tls_spki_sha256"] as? String == spki
        && value["authority_id"] as? String == authorityId.uuidString.lowercased()
        && value["authority_epoch"] as? Int == authorityEpoch
        && value["client_nonce"] as? String == base64url(clientNonce)
        && value["server_nonce"] as? String == base64url(serverNonce)
        && value["scope_profile"] as? String == "mac_app.v1"
}

private func randomData(count: Int) throws -> Data {
    var data = Data(count: count)
    let status = data.withUnsafeMutableBytes { bytes in
        guard let base = bytes.baseAddress else { return errSecParam }
        return SecRandomCopyBytes(kSecRandomDefault, bytes.count, base)
    }
    guard status == errSecSuccess else { throw SmokeError.randomness }
    return data
}

private func base64url(_ data: Data) -> String {
    data.base64EncodedString()
        .replacingOccurrences(of: "+", with: "-")
        .replacingOccurrences(of: "/", with: "_")
        .replacingOccurrences(of: "=", with: "")
}

private func fromBase64url(_ value: String) -> Data? {
    var value = value.replacingOccurrences(of: "-", with: "+")
        .replacingOccurrences(of: "_", with: "/")
    value += String(repeating: "=", count: (4 - value.count % 4) % 4)
    return Data(base64Encoded: value)
}

private enum SmokeError: Error {
    case usage, noCertificate, invalidResponse, rejected, randomness
}
