import Foundation

nonisolated enum FluidAudioModelPresence {
    static var parakeetEou320msDirectory: URL {
        FluidAudioCache.root
            .appendingPathComponent("parakeet-eou-streaming", isDirectory: true)
            .appendingPathComponent("parakeet-eou-streaming", isDirectory: true)
            .appendingPathComponent("320ms", isDirectory: true)
    }

    static let sentinelFiles: [String] = [
        "streaming_encoder.mlmodelc",
        "decoder.mlmodelc",
        "joint_decision.mlmodelc",
    ]

    static func isInstalled() -> Bool {
        let dir = self.parakeetEou320msDirectory
        let fm = FileManager.default
        for sentinel in self.sentinelFiles {
            let path = dir.appendingPathComponent(sentinel, isDirectory: true).path
            if !fm.fileExists(atPath: path) {
                return false
            }
        }
        return true
    }
}
