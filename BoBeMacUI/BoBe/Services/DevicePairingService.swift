import Foundation

actor DevicePairingService {
    static let shared = DevicePairingService()

    private struct PairingOrigin {
        let url: URL
        let mode: EndpointProfile.Mode
        let trustMode: EndpointProfile.TrustMode
        let transport: String
    }

    func pair(
        address: String,
        code: String,
        expectedMode: EndpointProfile.Mode
    ) async throws -> (EndpointProfile, String) {
        let pairingOrigin = try Self.canonicalOrigin(address, expectedMode: expectedMode)
        let outcome = try await PairingExchange.run(
            PairingTarget(
                url: pairingOrigin.url,
                transport: pairingOrigin.transport,
                trust: pairingOrigin.mode == .remote ? .firstUse : .loopback
            ),
            code: code,
            scopeProfile: "mac_app.v1",
            enrollment: PairingEnrollment(
                endpointId: SurfaceIdentity.current().uuidString,
                label: Host.current().localizedName ?? "Mac",
                protocolMin: Int(AttachWire.protocolMin),
                protocolMax: Int(AttachWire.protocolMax),
                capabilities: [
                    "microphone": true, "playback": true, "local_stt": true,
                    "local_tts": true, "screen_context": true,
                ]
            )
        )
        let reference = UUID().uuidString
        return (
            EndpointProfile(
                id: UUID(),
                mode: pairingOrigin.mode,
                baseURL: pairingOrigin.url,
                runtimeAuthorityId: outcome.runtimeAuthorityId,
                runtimeAuthorityEpoch: outcome.runtimeAuthorityEpoch,
                endpointId: outcome.endpointId,
                friendlyName: pairingOrigin.url.host ?? "BoBe",
                credentialReference: reference,
                trustMode: pairingOrigin.trustMode,
                tlsSPKISha256: outcome.tlsSPKISha256
            ),
            outcome.credential
        )
    }

    static func isValidAddress(_ value: String, expectedMode: EndpointProfile.Mode) -> Bool {
        (try? Self.canonicalOrigin(value, expectedMode: expectedMode)) != nil
    }

    private static func canonicalOrigin(
        _ value: String,
        expectedMode: EndpointProfile.Mode
    ) throws -> PairingOrigin {
        guard expectedMode == .attachedLocal || expectedMode == .remote else {
            throw DevicePairingError.invalidAddress
        }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        let defaultScheme = expectedMode == .attachedLocal ? "http" : "https"
        let candidate = trimmed.contains("://") ? trimmed : "\(defaultScheme)://\(trimmed)"
        guard var components = URLComponents(string: candidate),
              let scheme = components.scheme?.lowercased(),
              let host = components.host?.lowercased(),
              !host.isEmpty,
              components.user == nil,
              components.password == nil,
              components.query == nil,
              components.fragment == nil,
              components.path.isEmpty || components.path == "/"
        else { throw DevicePairingError.invalidAddress }
        let isLoopback = DaemonEndpoint.isLoopbackHost(host)
        let trustMode: EndpointProfile.TrustMode
        let transport: String
        switch expectedMode {
        case .attachedLocal where scheme == "http" && isLoopback:
            trustMode = .loopback
            transport = "loopback_http"
        case .remote where scheme == "https":
            trustMode = .pinnedSPKI
            transport = "https"
        default:
            throw DevicePairingError.invalidAddress
        }
        components.scheme = scheme
        components.host = host
        components.path = ""
        if (scheme == "https" && components.port == 443)
            || (scheme == "http" && components.port == 80) {
            components.port = nil
        }
        guard let url = components.url else { throw DevicePairingError.invalidAddress }
        return PairingOrigin(
            url: url,
            mode: expectedMode,
            trustMode: trustMode,
            transport: transport
        )
    }
}
