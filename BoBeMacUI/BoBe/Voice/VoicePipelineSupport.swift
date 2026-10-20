@preconcurrency import AVFoundation
import Foundation

nonisolated struct VoiceLoadTimeout: Error {}

nonisolated enum VoiceError: Error, CustomStringConvertible {
    case runtime(String)
    var description: String {
        switch self {
        case let .runtime(s): s
        }
    }
}

nonisolated func withVoiceLoadTimeout(
    _ timeout: Duration,
    _ operation: @escaping @Sendable () async throws -> Void
) async throws {
    try await withThrowingTaskGroup(of: Void.self) { group in
        group.addTask { try await operation() }
        group.addTask {
            try await Task.sleep(for: timeout)
            throw VoiceLoadTimeout()
        }
        _ = try await group.next()
        group.cancelAll()
    }
}

nonisolated struct VoiceSampleBuffer {
    private var storage: [Float] = []
    private var readIndex = 0

    var count: Int {
        self.storage.count - self.readIndex
    }

    mutating func append(contentsOf samples: [Float]) {
        self.storage.append(contentsOf: samples)
    }

    func firstFrame(count frameCount: Int) -> ArraySlice<Float>? {
        guard frameCount > 0, self.count >= frameCount else { return nil }
        return self.storage[self.readIndex ..< self.readIndex + frameCount]
    }

    @discardableResult
    mutating func consumeFirst(_ count: Int) -> Bool {
        guard count >= 0, count <= self.count else { return false }
        self.readIndex += count
        self.compactIfNeeded()
        return true
    }

    mutating func removeAll() {
        self.storage.removeAll(keepingCapacity: false)
        self.readIndex = 0
    }

    private mutating func compactIfNeeded() {
        guard self.readIndex >= 4_096,
              self.readIndex * 2 >= self.storage.count
        else { return }
        self.storage.removeFirst(self.readIndex)
        self.readIndex = 0
    }
}

nonisolated func voiceWsEndpoint(from baseURL: URL) -> URL? {
    guard var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false) else {
        return nil
    }
    let scheme = components.scheme?.lowercased()
    components.scheme = (scheme == "https") ? "wss" : "ws"
    let prefix = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    components.path = prefix.isEmpty ? "/voice/stream" : "/\(prefix)/voice/stream"
    return components.url
}

nonisolated func describe(_ format: AVAudioFormat) -> String {
    let fmtName: String = switch format.commonFormat {
    case .pcmFormatFloat32: "f32"
    case .pcmFormatFloat64: "f64"
    case .pcmFormatInt16: "i16"
    case .pcmFormatInt32: "i32"
    default: "?"
    }
    return "\(Int(format.sampleRate))Hz \(format.channelCount)ch \(fmtName)"
        + (format.isInterleaved ? "" : " (planar)")
}

nonisolated func convertSingleBuffer(
    _ converter: AVAudioConverter,
    source: AVAudioBuffer,
    into output: AVAudioPCMBuffer
) -> (status: AVAudioConverterOutputStatus, error: NSError?) {
    var fed = false
    var err: NSError?
    let status = converter.convert(to: output, error: &err) { _, statusPtr in
        guard !fed else {
            statusPtr.pointee = .noDataNow
            return nil
        }
        fed = true
        statusPtr.pointee = .haveData
        return source
    }
    return (status, err)
}

@available(macOS 27, *)
nonisolated func computePlaybackRms(_ buffer: AVReadOnlyAudioPCMBuffer) -> Float {
    let frames = buffer.frameLength
    guard frames > 0 else { return 0 }
    let stride = buffer.stride
    var sumSq: Double = 0
    switch buffer.channelData(0) {
    case let .float(span):
        for frame in 0 ..< frames {
            let s = Double(span[frame * stride])
            sumSq += s * s
        }
    case let .int16(span):
        for frame in 0 ..< frames {
            let s = Double(span[frame * stride]) / 32_768.0
            sumSq += s * s
        }
    case let .int32(span):
        for frame in 0 ..< frames {
            let s = Double(span[frame * stride]) / 2_147_483_648.0
            sumSq += s * s
        }
    @unknown default:
        return 0
    }
    return Float(sqrt(sumSq / Double(frames)))
}

nonisolated func computePlaybackRms(_ buffer: AVAudioPCMBuffer) -> Float {
    let frames = Int(buffer.frameLength)
    guard frames > 0 else { return 0 }
    var sumSq: Double = 0
    if let f32 = buffer.floatChannelData {
        let samples = f32[0]
        for i in 0 ..< frames {
            let s = Double(samples[i])
            sumSq += s * s
        }
    } else if let i16 = buffer.int16ChannelData {
        let samples = i16[0]
        for i in 0 ..< frames {
            let s = Double(samples[i]) / 32_768.0
            sumSq += s * s
        }
    } else {
        return 0
    }
    return Float(sqrt(sumSq / Double(frames)))
}
