import Foundation
import Testing

@Suite("Ephemeral preferences")
struct EphemeralUserDefaultsTests {
    @Test
    func removesTheOwnedDomainAndFileAfterItsLastConsumerReleasesIt() throws {
        var owner = EphemeralUserDefaults(prefix: "BoBeDefaultsCleanupTests")
        try #require(owner != nil)
        let suite = try #require(owner?.suiteName)
        let file = try #require(owner?.persistentFileURL)
        weak let released = owner
        var consumer: UserDefaults? = owner
        owner?.set("value", forKey: "sentinel")
        #expect(owner?.synchronize() == true)
        #expect(FileManager.default.fileExists(atPath: file.path))
        let reader = try #require(UserDefaults(suiteName: suite))
        #expect(reader.string(forKey: "sentinel") == "value")

        owner = nil
        #expect(released != nil)
        #expect(consumer?.string(forKey: "sentinel") == "value")
        consumer?.set("last write", forKey: "sentinel")
        consumer = nil

        #expect(released == nil)
        #expect(!FileManager.default.fileExists(atPath: file.path))
        #expect(!FileManager.default.fileExists(atPath: file.deletingLastPathComponent().path))
        #expect(reader.object(forKey: "sentinel") == nil)
        #expect(reader.synchronize())
        #expect(!FileManager.default.fileExists(atPath: file.path))
    }

    @Test
    func cleanupDoesNotAffectAnotherLiveSuite() throws {
        let first = try #require(EphemeralUserDefaults(prefix: "BoBeDefaultsCleanupTests"))
        let second = try #require(EphemeralUserDefaults(prefix: "BoBeDefaultsCleanupTests"))
        second.set("retained", forKey: "sentinel")
        #expect(second.synchronize())

        try first.cleanUp()
        try first.cleanUp()

        #expect(second.string(forKey: "sentinel") == "retained")
        #expect(FileManager.default.fileExists(atPath: second.persistentFileURL.path))
        try second.cleanUp()
        #expect(!FileManager.default.fileExists(atPath: second.persistentFileURL.path))
    }
}
