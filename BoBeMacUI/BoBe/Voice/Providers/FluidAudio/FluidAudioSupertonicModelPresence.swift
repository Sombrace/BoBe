import FluidAudio
import Foundation

enum FluidAudioSupertonicModelPresence {
    static func isInstalled() -> Bool {
        guard let cacheRoot = try? TtsCacheDirectory.ensure() else { return false }
        let modelDirectory = cacheRoot
            .appendingPathComponent("Models", isDirectory: true)
            .appendingPathComponent("supertonic-3", isDirectory: true)
        let required = ModelNames.Supertonic3.requiredFiles(veVariant: "ane-int4")
            .union([Supertonic3Voice.f1.fileName])
        return required.allSatisfy {
            FileManager.default.fileExists(
                atPath: modelDirectory.appendingPathComponent($0).path
            )
        }
    }
}
