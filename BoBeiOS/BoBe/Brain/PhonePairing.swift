import Foundation
import UIKit

enum PhonePairing {
    static let scopeProfile = "phone.v1"

    static func enrollment(endpointId: UUID, label: String) -> PairingEnrollment {
        let capabilities = PhoneAttach.capabilities
        return PairingEnrollment(
            endpointId: endpointId.uuidString,
            label: label,
            protocolMin: Int(AttachWire.protocolMin),
            protocolMax: Int(AttachWire.protocolMax),
            capabilities: [
                "microphone": capabilities.microphone,
                "playback": capabilities.playback,
                "local_stt": capabilities.localStt,
            ]
        )
    }

    static func pair(_ invite: PairingInvite) async throws -> BrainPairing {
        let enrollment = self.enrollment(endpointId: SurfaceIdentity.current(), label: UIDevice.current.name)
        var lastError: any Error = DevicePairingError.unreachable
        for origin in invite.origins {
            do {
                let outcome = try await PairingExchange.run(
                    PairingTarget(
                        url: origin,
                        transport: "https",
                        trust: invite.pin.map { .pinned($0) } ?? .firstUse
                    ),
                    code: invite.code,
                    scopeProfile: self.scopeProfile,
                    enrollment: enrollment
                )
                guard let pin = outcome.tlsSPKISha256 else { throw DevicePairingError.identityMismatch }
                return BrainPairing(
                    record: BrainRecord(
                        origins: [origin] + invite.origins.filter { $0 != origin },
                        pin: pin,
                        endpointId: outcome.endpointId,
                        authorityId: outcome.runtimeAuthorityId,
                        authorityEpoch: outcome.runtimeAuthorityEpoch
                    ),
                    credential: outcome.credential
                )
            } catch DevicePairingError.unreachable {
                lastError = DevicePairingError.unreachable
            }
        }
        throw lastError
    }
}
