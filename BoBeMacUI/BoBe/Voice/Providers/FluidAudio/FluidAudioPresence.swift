import FluidAudio
import Foundation

nonisolated protocol FluidAudioPresence {
    static func isInstalled() -> Bool
}

nonisolated extension FluidAudioModelPresence: FluidAudioPresence {}
nonisolated extension FluidAudioNemotronModelPresence: FluidAudioPresence {}

nonisolated func voiceSttProgress(_ progress: DownloadProgress) -> VoiceSttLoadProgress {
    let phase: VoiceSttLoadProgress.Phase = switch progress.phase {
    case .listing:
        .listing
    case let .downloading(completedFiles, totalFiles):
        .downloading(completedFiles: completedFiles, totalFiles: totalFiles)
    case let .compiling(modelName):
        .compiling(modelName: modelName)
    }
    return VoiceSttLoadProgress(
        fractionCompleted: progress.fractionCompleted,
        phase: phase
    )
}

nonisolated enum FluidAudioCache {
    static var root: URL {
        let appSupport = FileManager.default.urls(
            for: .applicationSupportDirectory,
            in: .userDomainMask
        ).first ?? FileManager.default.homeDirectoryForCurrentUser
        return appSupport
            .appendingPathComponent("FluidAudio", isDirectory: true)
            .appendingPathComponent("Models", isDirectory: true)
    }
}
