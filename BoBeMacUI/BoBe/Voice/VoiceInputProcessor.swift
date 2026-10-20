@preconcurrency import AVFoundation
import Foundation

nonisolated struct CapturedVoiceAudio: Sendable {
    let samples: [Float]
    let sampleRate: Double

    init?(buffer: AVAudioPCMBuffer) {
        let frameCount = Int(buffer.frameLength)
        guard frameCount > 0 else { return nil }
        let channels = max(1, Int(buffer.format.channelCount))
        let interleaved = buffer.format.isInterleaved

        if let data = buffer.floatChannelData?[0] {
            if interleaved, channels > 1 {
                self.samples = stride(from: 0, to: frameCount * channels, by: channels).map { data[$0] }
            } else {
                self.samples = Array(UnsafeBufferPointer(start: data, count: frameCount))
            }
        } else if let data = buffer.int16ChannelData?[0] {
            if interleaved, channels > 1 {
                self.samples = stride(from: 0, to: frameCount * channels, by: channels).map {
                    Float(data[$0]) / 32_768
                }
            } else {
                self.samples = UnsafeBufferPointer(start: data, count: frameCount).map {
                    Float($0) / 32_768
                }
            }
        } else {
            return nil
        }
        self.sampleRate = buffer.format.sampleRate
    }
}

nonisolated struct ProcessedVoiceAudio: Sendable {
    let samples16k: [Float]
    let rmsFramesDbfs: [Float]
}

actor VoiceInputProcessor {
    private static let outputSampleRate = Double(VoiceWire.captureSampleRate)
    private static let rmsFrameSamples = VoiceWire.captureFrameSamples

    private var converter: AVAudioConverter?
    private var converterSourceRate = 0.0
    private var rmsAccumulator: [Float] = []
    private var rmsReadIndex = 0

    func process(_ chunk: CapturedVoiceAudio) throws -> ProcessedVoiceAudio {
        let samples = try self.resampleTo16k(chunk)
        self.rmsAccumulator.append(contentsOf: samples)

        var levels: [Float] = []
        while self.rmsReadIndex + Self.rmsFrameSamples <= self.rmsAccumulator.count {
            let range = self.rmsReadIndex ..< self.rmsReadIndex + Self.rmsFrameSamples
            levels.append(Self.rmsDbfs(self.rmsAccumulator[range]))
            self.rmsReadIndex += Self.rmsFrameSamples
        }
        self.compactRmsAccumulatorIfNeeded()

        return ProcessedVoiceAudio(samples16k: samples, rmsFramesDbfs: levels)
    }

    func reset() {
        self.converter = nil
        self.converterSourceRate = 0
        self.rmsAccumulator.removeAll(keepingCapacity: false)
        self.rmsReadIndex = 0
    }

    private func resampleTo16k(_ chunk: CapturedVoiceAudio) throws -> [Float] {
        if chunk.sampleRate == Self.outputSampleRate {
            return chunk.samples
        }

        guard let sourceFormat = AVAudioFormat(
            commonFormat: .pcmFormatFloat32,
            sampleRate: chunk.sampleRate,
            channels: 1,
            interleaved: false
        ),
            let outputFormat = AVAudioFormat(
                commonFormat: .pcmFormatFloat32,
                sampleRate: Self.outputSampleRate,
                channels: 1,
                interleaved: false
            ),
            let sourceBuffer = AVAudioPCMBuffer(
                pcmFormat: sourceFormat,
                frameCapacity: AVAudioFrameCount(chunk.samples.count)
            ),
            let sourceData = sourceBuffer.floatChannelData?[0]
        else {
            throw VoiceInputProcessorError.audioConversionFailed
        }

        sourceBuffer.frameLength = AVAudioFrameCount(chunk.samples.count)
        chunk.samples.withUnsafeBufferPointer { samples in
            guard let base = samples.baseAddress else { return }
            sourceData.update(from: base, count: samples.count)
        }

        let converter: AVAudioConverter
        if let cached = self.converter, self.converterSourceRate == chunk.sampleRate {
            converter = cached
        } else {
            guard let fresh = AVAudioConverter(from: sourceFormat, to: outputFormat) else {
                throw VoiceInputProcessorError.audioConversionFailed
            }
            self.converter = fresh
            self.converterSourceRate = chunk.sampleRate
            converter = fresh
        }

        let capacity = AVAudioFrameCount(
            Double(chunk.samples.count) * Self.outputSampleRate / chunk.sampleRate
        ) + 16
        guard let output = AVAudioPCMBuffer(pcmFormat: outputFormat, frameCapacity: capacity) else {
            throw VoiceInputProcessorError.audioConversionFailed
        }
        let (status, error) = convertSingleBuffer(converter, source: sourceBuffer, into: output)
        guard status != .error, error == nil, let outputData = output.floatChannelData?[0] else {
            throw VoiceInputProcessorError.audioConversionFailed
        }
        return Array(UnsafeBufferPointer(start: outputData, count: Int(output.frameLength)))
    }

    private func compactRmsAccumulatorIfNeeded() {
        guard self.rmsReadIndex > 0 else { return }
        if self.rmsReadIndex >= 4_096 || self.rmsReadIndex * 2 >= self.rmsAccumulator.count {
            self.rmsAccumulator.removeFirst(self.rmsReadIndex)
            self.rmsReadIndex = 0
        }
    }

    private static func rmsDbfs(_ samples: ArraySlice<Float>) -> Float {
        guard !samples.isEmpty else { return -100 }
        var sumSquares: Double = 0
        for sample in samples {
            let value = Double(sample)
            sumSquares += value * value
        }
        let rms = sqrt(sumSquares / Double(samples.count))
        return rms > 1e-9 ? Float(20 * log10(rms)) : -100
    }
}

nonisolated enum VoiceInputProcessorError: Error {
    case audioConversionFailed
}
