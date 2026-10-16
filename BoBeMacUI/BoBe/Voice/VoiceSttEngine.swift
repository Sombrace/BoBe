import Foundation

protocol VoiceSttEngine: Sendable {
    func loadModels(
        onPartial: @escaping @Sendable (String) async -> Void,
        onEou: @escaping @Sendable (String) async -> Void,
        onFailure: @escaping @Sendable (String) async -> Void,
        onProgress: @escaping @Sendable (VoiceSttLoadProgress) -> Void
    ) async throws

    func cancelLoading() async

    func acceptSamples(_ samples: [Float]) async throws

    func reset() async throws

    func cleanup() async
}

enum VoiceSttCallbackEvent: Sendable {
    case partial(String)
    case endOfUtterance(String)
    case barrier(CheckedContinuation<Void, Never>)
}

struct VoiceSttLoadProgress: Sendable, Equatable {
    enum Phase: Sendable, Equatable {
        case listing
        case downloading(completedFiles: Int, totalFiles: Int)
        case compiling(modelName: String)
    }

    let fractionCompleted: Double
    let phase: Phase

    var percent: Int {
        max(0, min(100, Int((self.fractionCompleted * 100).rounded())))
    }
}
