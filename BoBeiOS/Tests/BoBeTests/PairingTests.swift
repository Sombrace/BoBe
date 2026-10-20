@testable import BoBe
import Foundation
import Security
import Testing

@Suite("iPhone pairing")
struct PairingTests {
    private nonisolated static let next = Fixture.pairing(
        "bobe_v2_second",
        origin: "https://second.bobe.test:8766",
        authority: Fixture.otherAuthority
    )

    private static var invite: PairingInvite? {
        PairingInvite(address: "second.bobe.test", code: "123456")
    }

    @Test
    func pairingAnotherBoBeForgetsThePreviousOne() async throws {
        let harness = Harness { _ in Self.next }
        defer { harness.stop() }
        let previous = harness.link()
        harness.link(Self.next).reply("POST /v1/hello", Fixture.hello(authority: Fixture.otherAuthority))

        await harness.model.pair(try #require(Self.invite))
        #expect(harness.vault.load() == Self.next)
        try await eventually { previous.requests("DELETE /endpoints/self").count == 1 }
        try await eventually { harness.vault.retiring().isEmpty }
        #expect(!harness.model.forgetPending)
        #expect(harness.link(Self.next).requests("DELETE /endpoints/self").isEmpty)
    }

    @Test
    func failedRevocationIsKeptAndRetried() async throws {
        let harness = Harness { _ in Self.next }
        defer { harness.stop() }
        let previous = harness.link()
        previous.fail("DELETE /endpoints/self")
        harness.link(Self.next).reply("POST /v1/hello", Fixture.hello(authority: Fixture.otherAuthority))

        await harness.model.pair(try #require(Self.invite))
        try await eventually { harness.model.forgetPending }
        #expect(harness.vault.retiring() == [Fixture.pairing()])
        #expect(harness.vault.load() == Self.next)

        previous.reply("DELETE /endpoints/self", Data())
        harness.model.retryForgetting()
        try await eventually { !harness.model.forgetPending }
        #expect(harness.vault.retiring().isEmpty)
        #expect(previous.requests("DELETE /endpoints/self").count == 2)
    }

    @Test
    func alreadyRevokedPairingCountsAsForgotten() async throws {
        let harness = Harness { _ in Self.next }
        defer { harness.stop() }
        harness.link().fail("DELETE /endpoints/self", .unauthorized)
        harness.link(Self.next).reply("POST /v1/hello", Fixture.hello(authority: Fixture.otherAuthority))

        await harness.model.pair(try #require(Self.invite))
        try await eventually { harness.link().requests("DELETE /endpoints/self").count == 1 }
        try await eventually { harness.vault.retiring().isEmpty }
        #expect(!harness.model.forgetPending)
    }

    @Test
    func failedSaveKeepsThePreviousPairing() async throws {
        let harness = Harness { _ in Self.next }
        defer { harness.stop() }
        harness.secrets.failWrites(to: "paired")

        await harness.model.pair(try #require(Self.invite))
        #expect(harness.model.pairing == .failed(L10n.tr("phone.pair.error.save")))
        #expect(harness.vault.load() == Fixture.pairing())
        #expect(harness.vault.retiring().isEmpty)
        await pause()
        #expect(harness.link().requests("DELETE /endpoints/self").isEmpty)
    }

    @Test
    func activePairingIsNeverRevoked() async throws {
        let harness = Harness()
        defer { harness.stop() }
        try harness.vault.saveRetiring([Fixture.pairing()])
        harness.model.retryForgetting()
        try await eventually { harness.vault.retiring().isEmpty }
        #expect(!harness.model.forgetPending)
        #expect(harness.link().requests("DELETE /endpoints/self").isEmpty)
    }

    @Test
    func unpairingForgetsThisIPhone() async throws {
        let harness = Harness()
        defer { harness.stop() }
        harness.model.unpair()
        #expect(harness.vault.load() == nil)
        try await eventually { harness.link().requests("DELETE /endpoints/self").count == 1 }
        try await eventually { harness.vault.retiring().isEmpty }
    }

    @Test
    func replacingAPairingUpdatesTheSavedItem() throws {
        let secrets = MemorySecrets()
        let vault = BrainVault(store: secrets)
        try vault.save(Fixture.pairing())
        try vault.save(Self.next)
        #expect(vault.load() == Self.next)
        #expect(secrets.calls == ["update paired", "add paired", "update paired"])
    }

    @Test
    func failedReplacementKeepsThePreviousPairing() throws {
        let secrets = MemorySecrets()
        let vault = BrainVault(store: secrets)
        try vault.save(Fixture.pairing())
        secrets.failWrites(to: "paired")
        #expect(throws: BrainVaultError.self) { try vault.save(Self.next) }
        #expect(vault.load() == Fixture.pairing())
    }

    @Test
    func pairingSavedByTheLastReleaseStillLoads() throws {
        let secrets = MemorySecrets()
        let stored: [String: Any] = [
            "record": [
                "origins": ["https://first.bobe.test:8766"],
                "pin": Data(repeating: 7, count: 32).base64EncodedString(),
            ],
            "credential": "bobe_v2_first",
        ]
        #expect(secrets.add(Fixture.json(stored), account: "paired") == errSecSuccess)
        let pairing = try #require(BrainVault(store: secrets).load())
        #expect(pairing == Fixture.pairing(authority: nil, epoch: nil))
    }
}
