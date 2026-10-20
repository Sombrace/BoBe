import FluidAudio
import Foundation

nonisolated enum FluidAudioModelPresence {
    static var parakeetEou320msDirectory: URL {
        FluidAudioCache.root
            .appendingPathComponent("parakeet-eou-streaming", isDirectory: true)
            .appendingPathComponent(Repo.parakeetEou320.folderName, isDirectory: true)
    }

    static func isInstalled() -> Bool {
        let directory = self.parakeetEou320msDirectory
        return ModelNames.ParakeetEOU.requiredModels.allSatisfy {
            FileManager.default.fileExists(atPath: directory.appendingPathComponent($0).path)
        }
    }
}
