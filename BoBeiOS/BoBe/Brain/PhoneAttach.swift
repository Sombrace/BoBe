import Foundation

enum PhoneAttach {
    enum Readiness: Equatable, Sendable {
        case ready
        case initializing
    }

    enum Failure: Error, Equatable {
        case notReady
        case incompatible
        case authorityChanged
        case missingScopes
    }

    struct Identity: Equatable, Sendable {
        let endpointId: UUID
        let authorityId: UUID
        let authorityEpoch: Int
    }

    enum Outcome: Equatable, Sendable {
        case attached(Readiness, Identity)
        case retry
        case removed
        case incompatible
    }

    static let scopes = [
        "conversation_read", "conversation_write", "goals_read", "goals_write", "memory_read", "events_subscribe",
    ]

    static let capabilities = SurfaceCapabilitiesWire(
        microphone: true,
        playback: false,
        wake: false,
        localStt: true,
        localTts: false,
        screenContext: false
    )

    static func request(surface: UUID, version: String) -> HelloRequest {
        HelloRequest(
            surfaceId: surface,
            surfaceKind: "phone",
            surfaceAppVersion: version.isEmpty ? "0" : version,
            protocolMin: AttachWire.protocolMin,
            protocolMax: AttachWire.protocolMax,
            capabilities: self.capabilities,
            requestedScopes: self.scopes
        )
    }

    static func attach(
        _ link: any BrainLink,
        record: BrainRecord,
        surface: UUID,
        version: String
    ) async -> Outcome {
        do {
            let hello: HelloResponse = try await link.send(
                AttachWire.path.split(separator: "/").map(String.init),
                body: self.request(surface: surface, version: version)
            )
            let readiness = try self.verify(hello, against: record)
            return .attached(
                readiness,
                Identity(
                    endpointId: surface,
                    authorityId: hello.runtimeAuthorityId,
                    authorityEpoch: hello.runtimeAuthorityEpoch
                )
            )
        } catch BrainError.unauthorized {
            return .removed
        } catch Failure.authorityChanged, Failure.missingScopes {
            return .removed
        } catch Failure.incompatible {
            return .incompatible
        } catch let BrainError.http(status, code) where status == 404 || code == "PROTOCOL_INCOMPATIBLE" {
            return .incompatible
        } catch {
            return .retry
        }
    }

    static func verify(_ hello: HelloResponse, against record: BrainRecord) throws -> Readiness {
        guard (AttachWire.protocolMin ... AttachWire.protocolMax).contains(hello.negotiatedProtocol) else {
            throw Failure.incompatible
        }
        if let authority = record.authorityId, authority != hello.runtimeAuthorityId {
            throw Failure.authorityChanged
        }
        if let epoch = record.authorityEpoch, hello.runtimeAuthorityEpoch < epoch {
            throw Failure.authorityChanged
        }
        guard Set(self.scopes).isSubset(of: hello.grantedScopes) else { throw Failure.missingScopes }
        switch hello.readiness.state {
        case .ready: return .ready
        case .initializing: return .initializing
        case .notReady, .unknown: throw Failure.notReady
        }
    }
}

extension BrainRecord {
    func adopting(_ identity: PhoneAttach.Identity) -> BrainRecord {
        var record = self
        record.endpointId = self.endpointId ?? identity.endpointId
        record.authorityId = identity.authorityId
        record.authorityEpoch = max(self.authorityEpoch ?? identity.authorityEpoch, identity.authorityEpoch)
        return record
    }
}
