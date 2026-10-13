import FluidAudio
import Foundation

protocol FluidAudioPresence {
    static func isInstalled() -> Bool
}

extension FluidAudioModelPresence: FluidAudioPresence {}
extension FluidAudioNemotronModelPresence: FluidAudioPresence {}

func voiceSttProgress(_ progress: DownloadProgress) -> VoiceSttLoadProgress {
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

enum FluidAudioCache {
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
