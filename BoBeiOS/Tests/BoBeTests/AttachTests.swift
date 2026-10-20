@testable import BoBe
import Foundation
import Testing

@Suite("iPhone attach handshake")
struct AttachTests {
    @Test
    func helloIdentifiesThePhoneBeforeStreaming() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        try await harness.connect()

        #expect(link.requests.first.map { "\($0.method) \($0.path)" } == "POST /v1/hello")
        let hello = try #require(link.requests("POST /v1/hello").first).json
        #expect(hello["surface_kind"] as? String == "phone")
        #expect((hello["surface_id"] as? String).flatMap(UUID.init(uuidString:)) == Fixture.endpoint)
        #expect(Set(hello["requested_scopes"] as? [String] ?? []) == Set(PhoneAttach.scopes))
        let capabilities = try #require(hello["capabilities"] as? [String: Bool])
        #expect(capabilities["playback"] == false)
        #expect(capabilities["microphone"] == true)
        #expect(capabilities["local_stt"] == true)
    }

    @Test
    func anotherAuthorityEndsThePairing() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        link.reply("POST /v1/hello", Fixture.hello(authority: Fixture.otherAuthority))
        harness.model.start()
        try await eventually { harness.model.link == .removed }
        #expect(link.cursors.isEmpty)
    }

    @Test
    func authorityRollbackEndsThePairing() async throws {
        let harness = Harness(paired: Fixture.pairing(epoch: 2))
        defer { harness.stop() }
        let link = harness.link()
        link.reply("POST /v1/hello", Fixture.hello(epoch: 1))
        harness.model.start()
        try await eventually { harness.model.link == .removed }
        #expect(link.cursors.isEmpty)
    }

    @Test
    func missingScopesEndThePairing() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        link.reply("POST /v1/hello", Fixture.hello(scopes: Fixture.phoneScopes.filter { $0 != "events_subscribe" }))
        harness.model.start()
        try await eventually { harness.model.link == .removed }
        #expect(link.cursors.isEmpty)
    }

    @Test
    func incompatibleProtocolAsksForAnUpdate() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        link.fail("POST /v1/hello", .http(409, code: "PROTOCOL_INCOMPATIBLE"))
        harness.model.start()
        try await eventually { harness.model.link == .incompatible }
        #expect(link.cursors.isEmpty)
        #expect(harness.model.status == L10n.tr("phone.status.update"))
    }

    @Test
    func initializingBoBeIsConnectedButCannotChat() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        link.reply("POST /v1/hello", Fixture.hello(readiness: "initializing"))
        harness.model.start()
        try await eventually { harness.model.link == .settingUp }
        #expect(link.cursors.count == 1)
        #expect(harness.model.isConnected)
        #expect(harness.model.status == L10n.tr("phone.status.setting_up"))

        harness.model.send("Hello?")
        #expect(harness.model.thread.last?.delivery == .failed)
        #expect(link.requests("POST /message").isEmpty)

        link.reply("POST /v1/hello", Fixture.hello())
        try await eventually(.seconds(20)) { harness.model.link == .online }
        #expect(link.cursors.count == 1)
    }

    @Test
    func notReadyBoBeIsOfflineAndRetried() async throws {
        let harness = Harness()
        defer { harness.stop() }
        let link = harness.link()
        link.reply("POST /v1/hello", Fixture.hello(readiness: "not_ready"))
        harness.model.start()
        try await eventually { harness.model.link == .offline }
        #expect(link.cursors.isEmpty)

        link.reply("POST /v1/hello", Fixture.hello())
        try await eventually(.seconds(20)) { harness.model.link == .online }
    }

    @Test
    func olderPairingLearnsItsAuthority() async throws {
        let legacy = Fixture.pairing(authority: nil, epoch: nil)
        let harness = Harness(paired: legacy)
        defer { harness.stop() }
        try await harness.connect()
        let saved = try #require(harness.vault.load())
        #expect(saved.record.authorityId == Fixture.authority)
        #expect(saved.record.authorityEpoch == 1)
        #expect(saved.record.endpointId != nil)
        #expect(saved.credential == legacy.credential)
    }

    @Test
    func phoneDeclaresNoPlayback() {
        let enrollment = PhonePairing.enrollment(endpointId: Fixture.endpoint, label: "Test iPhone")
        #expect(enrollment.capabilities == ["microphone": true, "playback": false, "local_stt": true])
        #expect(enrollment.endpointId == Fixture.endpoint.uuidString)
    }
}
