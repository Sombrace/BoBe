@preconcurrency import AVFoundation
import Foundation

final class OpusDecoder {
    private let converter: AVAudioConverter
    private let inputFormat: AVAudioFormat

    enum DecodeError: Error {
        case invalidInputFormat
        case converterUnavailable
        case decodeFailed(NSError?)
    }

    init(outputFormat: AVAudioFormat) throws(DecodeError) {
        var asbd = AudioStreamBasicDescription(
            mSampleRate: outputFormat.sampleRate,
            mFormatID: kAudioFormatOpus,
            mFormatFlags: 0,
            mBytesPerPacket: 0,
            mFramesPerPacket: 0,
            mBytesPerFrame: 0,
            mChannelsPerFrame: outputFormat.channelCount,
            mBitsPerChannel: 0,
            mReserved: 0
        )
        guard let opus = AVAudioFormat(streamDescription: &asbd) else {
            throw DecodeError.invalidInputFormat
        }
        guard let converter = AVAudioConverter(from: opus, to: outputFormat) else {
            throw DecodeError.converterUnavailable
        }
        self.inputFormat = opus
        self.converter = converter
    }

    func decode(_ packet: RawSpan, to buffer: AVAudioPCMBuffer) throws(DecodeError) {
        guard !packet.isEmpty else { return }
        let byteCount = packet.byteCount

        let compressed = AVAudioCompressedBuffer(
            format: self.inputFormat,
            packetCapacity: 1,
            maximumPacketSize: byteCount
        )
        packet.withUnsafeBytes { source in
            guard let base = source.baseAddress else { return }
            compressed.data.copyMemory(from: base, byteCount: byteCount)
        }
        compressed.byteLength = UInt32(byteCount)
        compressed.packetCount = 1
        compressed.packetDescriptions?.pointee = AudioStreamPacketDescription(
            mStartOffset: 0,
            mVariableFramesInPacket: 0,
            mDataByteSize: UInt32(byteCount)
        )

        let (status, convErr) = convertSingleBuffer(self.converter, source: compressed, into: buffer)
        if status == .error {
            throw DecodeError.decodeFailed(convErr)
        }
    }
}
