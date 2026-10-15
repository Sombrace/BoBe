import Foundation

struct AttachResult: Sendable {
    let hello: HelloResponse
}

actor EndpointAttachService {
    static let shared = EndpointAttachService()
    static let surfaceIdKey = SurfaceIdentity.defaultsKey
    static let requiredScopes = [
        "conversation_read", "conversation_write", "memory_read", "memory_write",
        "goals_read", "goals_write", "settings_read", "settings_write", "provider_admin",
        "voice_session", "capture_submit", "body_admin", "events_subscribe",
    ]

    func attachConfiguredEndpoint(
        store: EndpointStore = .shared,
        selection expectedSelection: EndpointSelection? = nil
    ) async throws -> AttachResult? {
        let selection = expectedSelection ?? store.selection
        guard let profile = store.profile(for: selection) else {
            if store.selection != selection { throw CancellationError() }
            return nil
        }
        let result = try await self.attach(
            baseURL: selection.endpoint.baseURL,
            endpoint: selection.endpoint,
            expectedAuthorityId: profile.runtimeAuthorityId,
            expectedAuthorityEpoch: profile.runtimeAuthorityEpoch
        )
        try store.updateNegotiatedIdentity(
            profileID: profile.id,
            authorityID: result.hello.runtimeAuthorityId,
            authorityEpoch: result.hello.runtimeAuthorityEpoch,
            expectedSelection: selection
        )
        return result
    }

    func attach(profile: EndpointProfile, credential: String?) async throws -> AttachResult {
        guard let endpoint = profile.daemonEndpoint(credential: credential) else {
            throw AttachError.unauthorized
        }
        return try await self.attach(
            baseURL: profile.baseURL,
            endpoint: endpoint,
            expectedAuthorityId: profile.runtimeAuthorityId,
            expectedAuthorityEpoch: profile.runtimeAuthorityEpoch
        )
    }

    private func attach(
        baseURL: URL,
        endpoint: DaemonEndpoint,
        expectedAuthorityId: UUID?,
        expectedAuthorityEpoch: Int?
    ) async throws -> AttachResult {
        let requestBody = HelloRequest(
            surfaceId: SurfaceIdentity.current(),
            surfaceKind: "mac_app",
            surfaceAppVersion: Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0",
            protocolMin: AttachWire.protocolMin,
            protocolMax: AttachWire.protocolMax,
            capabilities: SurfaceCapabilitiesWire(
                microphone: true,
                playback: true,
                wake: false,
                localStt: true,
                localTts: true,
                screenContext: true
            ),
            requestedScopes: Self.requiredScopes
        )
        var request = URLRequest(url: baseURL.appendingPathComponent("v1/hello"))
        request.httpMethod = "POST"
        request.timeoutInterval = 10
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try JSONEncoder().encode(requestBody)
        endpoint.authorize(&request)
        let session = DaemonTransportPolicy.session(for: endpoint, purpose: .attach)
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse else { throw AttachError.unreachable }
        switch http.statusCode {
        case 200 ... 299:
            let hello = try JSONDecoder().decode(HelloResponse.self, from: data)
            try Self.verify(
                hello,
                expectedAuthorityId: expectedAuthorityId,
                expectedAuthorityEpoch: expectedAuthorityEpoch
            )
            return AttachResult(hello: hello)
        case 404, 409:
            throw AttachError.protocolIncompatible
        case 401:
            throw AttachError.unauthorized
        default:
            throw AttachError.unreachable
        }
    }

    static func verify(
        _ hello: HelloResponse,
        expectedAuthorityId: UUID?,
        expectedAuthorityEpoch: Int?
    ) throws {
        guard hello.negotiatedProtocol >= AttachWire.protocolMin,
              hello.negotiatedProtocol <= AttachWire.protocolMax
        else { throw AttachError.protocolIncompatible }
        guard hello.readiness.state.isServing else {
            throw AttachError.notReady(reasons: hello.readiness.blockingReasons)
        }
        if let expected = expectedAuthorityId, expected != hello.runtimeAuthorityId {
            throw AttachError.authorityMismatch(expected: expected, actual: hello.runtimeAuthorityId)
        }
        if let expectedAuthorityEpoch, hello.runtimeAuthorityEpoch < expectedAuthorityEpoch {
            throw AttachError.authorityRollback(
                expected: expectedAuthorityEpoch,
                actual: hello.runtimeAuthorityEpoch
            )
        }
        let granted = Set(hello.grantedScopes)
        let missing = Self.requiredScopes.filter { !granted.contains($0) }
        guard missing.isEmpty else { throw AttachError.missingScopes(missing) }
    }
}

enum RuntimeRecoveryAction: Sendable, Equatable {
    case retryConnection
    case pairAgain
    case chooseRuntime
}

enum RuntimeIssueKind: Sendable, Equatable {
    case unreachable
    case notReady
    case credentialRejected
    case secureVerificationFailed
    case authorityChanged
    case protocolIncompatible
    case accessIncomplete
    case localMaterial
    case managedStartup
    case other
}

struct RuntimeIssue: Sendable, Equatable {
    let kind: RuntimeIssueKind
    let message: String
    let primaryAction: RuntimeRecoveryAction
    let expertDetails: String?

    init(_ error: any Error) {
        let details = error.localizedDescription
        switch error {
        case AttachError.unreachable:
            self.init(
                kind: .unreachable,
                message: L10n.tr("runtime.issue.unreachable"),
                primaryAction: .retryConnection,
                expertDetails: details
            )
        case AttachError.unauthorized:
            self.init(
                kind: .credentialRejected,
                message: L10n.tr("runtime.issue.credential_rejected"),
                primaryAction: .pairAgain,
                expertDetails: nil
            )
        case AttachError.protocolIncompatible:
            self.init(
                kind: .protocolIncompatible,
                message: L10n.tr("runtime.issue.protocol_incompatible"),
                primaryAction: .chooseRuntime,
                expertDetails: details
            )
        case AttachError.notReady:
            self.init(
                kind: .notReady,
                message: L10n.tr("runtime.issue.not_ready"),
                primaryAction: .retryConnection,
                expertDetails: details
            )
        case AttachError.authorityMismatch, AttachError.authorityRollback:
            self.init(
                kind: .authorityChanged,
                message: L10n.tr("runtime.issue.authority_changed"),
                primaryAction: .pairAgain,
                expertDetails: details
            )
        case AttachError.missingScopes:
            self.init(
                kind: .accessIncomplete,
                message: L10n.tr("runtime.issue.access_incomplete"),
                primaryAction: .pairAgain,
                expertDetails: details
            )
        case is EndpointStoreError, is EndpointCredentialError:
            self.init(
                kind: .localMaterial,
                message: L10n.tr("runtime.issue.local_material"),
                primaryAction: .pairAgain,
                expertDetails: details
            )
        case is BackendServiceError:
            self.init(
                kind: .managedStartup,
                message: L10n.tr("runtime.issue.managed_startup"),
                primaryAction: .retryConnection,
                expertDetails: details
            )
        default:
            let nsError = error as NSError
            if Self.isSecureVerificationFailure(nsError) {
                self.init(
                    kind: .secureVerificationFailed,
                    message: L10n.tr("runtime.issue.secure_verification"),
                    primaryAction: .pairAgain,
                    expertDetails: details
                )
            } else if nsError.domain == NSURLErrorDomain {
                self.init(
                    kind: .unreachable,
                    message: L10n.tr("runtime.issue.unreachable"),
                    primaryAction: .retryConnection,
                    expertDetails: details
                )
            } else {
                self.init(
                    kind: .other,
                    message: details,
                    primaryAction: .retryConnection,
                    expertDetails: details
                )
            }
        }
    }

    init(
        kind: RuntimeIssueKind,
        message: String,
        primaryAction: RuntimeRecoveryAction,
        expertDetails: String?
    ) {
        self.kind = kind
        self.message = message
        self.primaryAction = primaryAction
        self.expertDetails = expertDetails
    }

    static func isSecureVerificationFailure(_ error: NSError) -> Bool {
        guard error.domain == NSURLErrorDomain else { return false }
        return [
            NSURLErrorServerCertificateUntrusted,
            NSURLErrorServerCertificateHasBadDate,
            NSURLErrorServerCertificateNotYetValid,
            NSURLErrorServerCertificateHasUnknownRoot,
            NSURLErrorClientCertificateRejected,
            NSURLErrorClientCertificateRequired,
            NSURLErrorSecureConnectionFailed,
        ].contains(error.code)
    }
}

enum AttachError: LocalizedError {
    case unreachable
    case unauthorized
    case protocolIncompatible
    case notReady(reasons: [String])
    case authorityMismatch(expected: UUID, actual: UUID)
    case authorityRollback(expected: Int, actual: Int)
    case missingScopes([String])

    var errorDescription: String? {
        switch self {
        case .unreachable: L10n.tr("runtime.issue.unreachable")
        case .unauthorized: L10n.tr("runtime.issue.credential_rejected")
        case .protocolIncompatible: L10n.tr("runtime.issue.protocol_incompatible")
        case let .notReady(reasons):
            if reasons.isEmpty {
                L10n.tr("runtime.issue.not_ready")
            } else {
                L10n.tr(
                    "runtime.issue.not_ready_details_format",
                    reasons.joined(separator: ", ").replacingOccurrences(of: "_", with: " ")
                )
            }
        case let .authorityMismatch(expected, actual):
            L10n.tr(
                "runtime.issue.authority_mismatch_details_format",
                expected.uuidString,
                actual.uuidString
            )
        case let .authorityRollback(expected, actual):
            L10n.tr("runtime.issue.authority_rollback_details_format", expected, actual)
        case let .missingScopes(scopes):
            L10n.tr(
                "runtime.issue.missing_access_details_format",
                scopes.joined(separator: ", ").replacingOccurrences(of: "_", with: " ")
            )
        }
    }
}
