import Foundation

nonisolated enum FluidAudioNemotronModelPresence {
    static var modelDirectory: URL {
        FluidAudioCache.root
            .appendingPathComponent("nemotron-multilingual", isDirectory: true)
            .appendingPathComponent("multilingual", isDirectory: true)
            .appendingPathComponent("\(VoiceSttTuning.nemotronChunkMs)ms", isDirectory: true)
    }

    static func isInstalled() -> Bool {
        let directory = self.modelDirectory
        let fileManager = FileManager.default
        let required = [
            "metadata.json",
            "tokenizer.json",
            "encoder.mlmodelc",
        ]
        guard required.allSatisfy({
            fileManager.fileExists(atPath: directory.appendingPathComponent($0).path)
        })
        else {
            return false
        }

        let fusedDecoders = [
            "decoder_joint_argmax.mlmodelc",
            "decoder_joint_noencproj.mlmodelc",
            "decoder_joint.mlmodelc",
        ]
        if fusedDecoders.contains(where: {
            fileManager.fileExists(atPath: directory.appendingPathComponent($0).path)
        }) {
            return true
        }

        return fileManager.fileExists(
            atPath: directory.appendingPathComponent("decoder.mlmodelc").path
        ) && fileManager.fileExists(
            atPath: directory.appendingPathComponent("joint.mlmodelc").path
        )
    }
}
