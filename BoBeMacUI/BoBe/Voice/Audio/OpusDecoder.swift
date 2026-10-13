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

    init(outputFormat: AVAudioFormat) throws {
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

    func decode(_ packet: UnsafeBufferPointer<UInt8>, to buffer: AVAudioPCMBuffer) throws {
        guard let base = packet.baseAddress, !packet.isEmpty else { return }
        let byteCount = packet.count

        let compressed = AVAudioCompressedBuffer(
            format: self.inputFormat,
            packetCapacity: 1,
            maximumPacketSize: byteCount
        )
        memcpy(compressed.data, base, byteCount)
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
