import Foundation

nonisolated struct SpeechPause {
    static let frameSamples = VoiceWire.captureFrameSamples
    static let frameDuration = Duration.milliseconds(1_000 * VoiceWire.captureFrameSamples / VoiceWire.captureSampleRate)

    private(set) var heardSpeech = false
    private(set) var silence = Duration.zero
    private var noiseFloorDbfs: Float = -60
    private var remainder: [Float] = []

    mutating func observe(_ samples: [Float]) {
        self.remainder.append(contentsOf: samples)
        var start = 0
        while start + Self.frameSamples <= self.remainder.count {
            self.observeFrame(levelDbfs: Self.levelDbfs(self.remainder[start ..< start + Self.frameSamples]))
            start += Self.frameSamples
        }
        self.remainder.removeFirst(start)
    }

    func reached(_ pause: Duration) -> Bool {
        self.heardSpeech && self.silence >= pause
    }

    mutating func reset() {
        self.heardSpeech = false
        self.silence = .zero
        self.remainder.removeAll(keepingCapacity: true)
    }

    private mutating func observeFrame(levelDbfs: Float) {
        if levelDbfs > max(self.noiseFloorDbfs + 12, -50) {
            self.heardSpeech = true
            self.silence = .zero
            return
        }
        self.silence += Self.frameDuration
        self.noiseFloorDbfs = min(-40, max(-90, self.noiseFloorDbfs * 0.9 + levelDbfs * 0.1))
    }

    static func levelDbfs(_ samples: ArraySlice<Float>) -> Float {
        guard !samples.isEmpty else { return -100 }
        let energy = samples.reduce(into: Float.zero) { $0 += $1 * $1 }
        let rms = (energy / Float(samples.count)).squareRoot()
        return rms > 1e-5 ? 20 * log10(rms) : -100
    }
}
