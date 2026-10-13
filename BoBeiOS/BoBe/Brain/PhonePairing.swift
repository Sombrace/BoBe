import Foundation
import UIKit

enum PhonePairing {
    static let scopeProfile = "phone.v1"

    static func pair(_ invite: PairingInvite) async throws -> BrainPairing {
        let label = await MainActor.run { UIDevice.current.name }
        let enrollment = PairingEnrollment(
            endpointId: SurfaceIdentity.current().uuidString,
            label: label,
            protocolMin: Int(AttachWire.protocolMin),
            protocolMax: Int(AttachWire.protocolMax),
            capabilities: ["microphone": true, "playback": true, "local_stt": true]
        )
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
