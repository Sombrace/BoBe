import AVFoundation
import CoreGraphics
import CoreImage
import CoreML
import CryptoKit
import Darwin
import Foundation
import ImageIO
import Vision

private let toolVersion = "1.1.0"
private let schemaVersion = 1
private let authorizationTimeoutSeconds = 300.0
private let minimumWarmupMilliseconds = 250
private let maximumWarmupMilliseconds = 10_000
private let cameraSettleTimeoutSeconds = 2.0
private let captureStartTimeoutSeconds = 5.0
private let captureBurstTimeoutSeconds = 5.0
private let captureBurstSize = 8
private let textRecognitionRevision = VNRecognizeTextRequestRevision3
private let rectangleRecognitionRevision = VNDetectRectanglesRequestRevision1

private struct ToolError: Error {
    let code: String
    let message: String
    let exitCode: Int32
    let details: [String: Any]

    init(
        _ code: String,
        _ message: String,
        exitCode: Int32 = 2,
        details: [String: Any] = [:]
    ) {
        self.code = code
        self.message = message
        self.exitCode = exitCode
        self.details = details
    }
}

private struct CommandResult {
    let value: [String: Any]
    let exitCode: Int32

    init(_ value: [String: Any], exitCode: Int32 = 0) {
        self.value = value
        self.exitCode = exitCode
    }
}

private struct ParsedOptions {
    let values: [String: String]
    let flags: Set<String>

    init(
        arguments: ArraySlice<String>,
        valueNames: Set<String>,
        flagNames: Set<String>
    ) throws {
        var parsedValues: [String: String] = [:]
        var parsedFlags = Set<String>()
        var index = arguments.startIndex

        while index < arguments.endIndex {
            let argument = arguments[index]
            guard argument.hasPrefix("--") else {
                throw ToolError(
                    "INVALID_ARGUMENT",
                    "Unexpected positional argument: \(argument)"
                )
            }
            guard !argument.contains("=") else {
                throw ToolError(
                    "INVALID_ARGUMENT",
                    "Use separate tokens for option names and values: \(argument)"
                )
            }
            let name = String(argument.dropFirst(2))
            if flagNames.contains(name) {
                guard !parsedFlags.contains(name) else {
                    throw ToolError("INVALID_ARGUMENT", "Duplicate option: --\(name)")
                }
                parsedFlags.insert(name)
                index = arguments.index(after: index)
                continue
            }
            guard valueNames.contains(name) else {
                throw ToolError("INVALID_ARGUMENT", "Unknown option: --\(name)")
            }
            guard parsedValues[name] == nil else {
                throw ToolError("INVALID_ARGUMENT", "Duplicate option: --\(name)")
            }
            let valueIndex = arguments.index(after: index)
            guard valueIndex < arguments.endIndex else {
                throw ToolError("INVALID_ARGUMENT", "Missing value for --\(name)")
            }
            let value = arguments[valueIndex]
            guard !value.isEmpty, !value.hasPrefix("--") else {
                throw ToolError("INVALID_ARGUMENT", "Missing value for --\(name)")
            }
            parsedValues[name] = value
            index = arguments.index(after: valueIndex)
        }

        values = parsedValues
        flags = parsedFlags
    }

    func requireValue(_ name: String) throws -> String {
        guard let value = values[name] else {
            throw ToolError("INVALID_ARGUMENT", "Missing required option: --\(name)")
        }
        return value
    }

}

private final class FrameCollector: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate {
    struct Frame {
        let sampleBuffer: CMSampleBuffer
        let capturedAt: Date
    }

    private let condition = NSCondition()
    private var frames: [Frame] = []
    private var accepting = false

    func beginBurst() {
        condition.lock()
        frames.removeAll(keepingCapacity: true)
        accepting = true
        condition.unlock()
    }

    func waitForBurst(count: Int, timeout: TimeInterval) -> [Frame] {
        let deadline = Date(timeIntervalSinceNow: timeout)
        condition.lock()
        defer {
            accepting = false
            condition.unlock()
        }
        while frames.count < count {
            if !condition.wait(until: deadline) {
                break
            }
        }
        return frames
    }

    func captureOutput(
        _ output: AVCaptureOutput,
        didOutput sampleBuffer: CMSampleBuffer,
        from connection: AVCaptureConnection
    ) {
        condition.lock()
        if accepting, frames.count < captureBurstSize {
            frames.append(Frame(sampleBuffer: sampleBuffer, capturedAt: Date()))
            condition.broadcast()
        }
        condition.unlock()
    }
}

private func JSONData(_ value: Any) throws -> Data {
    guard JSONSerialization.isValidJSONObject(value) else {
        throw ToolError(
            "INTERNAL_ERROR",
            "The result could not be represented as JSON",
            exitCode: 70
        )
    }
    var data = try JSONSerialization.data(
        withJSONObject: value,
        options: [.sortedKeys, .withoutEscapingSlashes]
    )
    data.append(0x0A)
    return data
}

private func writeJSON(_ value: Any, to handle: FileHandle) {
    do {
        try handle.write(contentsOf: JSONData(value))
    } catch {
        let fallback = "{\"error\":{\"code\":\"INTERNAL_ERROR\",\"message\":\"Failed to encode JSON\"},\"ok\":false}\n"
        try? handle.write(contentsOf: Data(fallback.utf8))
    }
}

private func successEnvelope(
    command: String,
    fields: [String: Any] = [:]
) -> [String: Any] {
    var value: [String: Any] = [
        "command": command,
        "ok": true,
        "schema_version": schemaVersion,
        "version": toolVersion,
    ]
    for (key, field) in fields {
        value[key] = field
    }
    return value
}

private func errorEnvelope(_ error: ToolError) -> [String: Any] {
    var body: [String: Any] = [
        "code": error.code,
        "message": error.message,
    ]
    if !error.details.isEmpty {
        body["details"] = error.details
    }
    return [
        "error": body,
        "ok": false,
        "schema_version": schemaVersion,
        "version": toolVersion,
    ]
}

private func authorizationName(_ status: AVAuthorizationStatus) -> String {
    switch status {
    case .notDetermined:
        return "not_determined"
    case .restricted:
        return "restricted"
    case .denied:
        return "denied"
    case .authorized:
        return "authorized"
    @unknown default:
        return "unknown"
    }
}

private func videoDevices() -> [AVCaptureDevice] {
    let discovery = AVCaptureDevice.DiscoverySession(
        deviceTypes: [
            .builtInWideAngleCamera,
            .continuityCamera,
            .deskViewCamera,
            .external,
        ],
        mediaType: .video,
        position: .unspecified
    )
    return discovery.devices.sorted {
        if $0.localizedName == $1.localizedName {
            return $0.uniqueID < $1.uniqueID
        }
        return $0.localizedName < $1.localizedName
    }
}

private func fourCharacterCode(_ value: FourCharCode) -> String {
    let bytes: [UInt8] = [
        UInt8((value >> 24) & 0xFF),
        UInt8((value >> 16) & 0xFF),
        UInt8((value >> 8) & 0xFF),
        UInt8(value & 0xFF),
    ]
    if bytes.allSatisfy({ $0 >= 0x20 && $0 <= 0x7E }) {
        return String(bytes: bytes, encoding: .ascii) ?? String(format: "0x%08x", value)
    }
    return String(format: "0x%08x", value)
}

private func formatDescription(
    _ format: AVCaptureDevice.Format,
    activeFormat: AVCaptureDevice.Format?
) -> [String: Any] {
    let dimensions = CMVideoFormatDescriptionGetDimensions(format.formatDescription)
    let ranges = format.videoSupportedFrameRateRanges.map { range in
        [
            "maximum": range.maxFrameRate,
            "minimum": range.minFrameRate,
        ]
    }
    return [
        "frame_rate_ranges": ranges,
        "height": Int(dimensions.height),
        "is_active": activeFormat.map { format.isEqual($0) } ?? false,
        "media_subtype": fourCharacterCode(
            CMFormatDescriptionGetMediaSubType(format.formatDescription)
        ),
        "width": Int(dimensions.width),
    ]
}

private func maximumFrameRate(_ format: AVCaptureDevice.Format) -> Double {
    format.videoSupportedFrameRateRanges.map(\.maxFrameRate).max() ?? 0
}

private func preferredFrameRate(_ format: AVCaptureDevice.Format) -> Double {
    let ranges = format.videoSupportedFrameRateRanges
    if ranges.contains(where: { $0.minFrameRate <= 30 && $0.maxFrameRate >= 30 }) {
        return 30
    }
    return ranges.map(\.maxFrameRate).max() ?? 0
}

private func preferredFrameDuration(_ format: AVCaptureDevice.Format) -> CMTime? {
    let ranges = format.videoSupportedFrameRateRanges
    let selected = ranges.first(where: {
        $0.minFrameRate <= 30 && $0.maxFrameRate >= 30
    }) ?? ranges.max(by: { $0.maxFrameRate < $1.maxFrameRate })
    return selected?.minFrameDuration
}

private func selectedCameraFormat(_ device: AVCaptureDevice) -> AVCaptureDevice.Format? {
    device.formats.sorted { first, second in
        let firstDimensions = CMVideoFormatDescriptionGetDimensions(first.formatDescription)
        let secondDimensions = CMVideoFormatDescriptionGetDimensions(second.formatDescription)
        let firstLandscape = firstDimensions.width > firstDimensions.height
        let secondLandscape = secondDimensions.width > secondDimensions.height
        if firstLandscape != secondLandscape {
            return firstLandscape
        }
        let firstPixels = Int64(firstDimensions.width) * Int64(firstDimensions.height)
        let secondPixels = Int64(secondDimensions.width) * Int64(secondDimensions.height)
        if firstPixels != secondPixels {
            return firstPixels > secondPixels
        }
        let firstRateDistance = abs(maximumFrameRate(first) - 30)
        let secondRateDistance = abs(maximumFrameRate(second) - 30)
        if firstRateDistance != secondRateDistance {
            return firstRateDistance < secondRateDistance
        }
        if firstDimensions.width != secondDimensions.width {
            return firstDimensions.width > secondDimensions.width
        }
        if firstDimensions.height != secondDimensions.height {
            return firstDimensions.height > secondDimensions.height
        }
        return CMFormatDescriptionGetMediaSubType(first.formatDescription)
            < CMFormatDescriptionGetMediaSubType(second.formatDescription)
    }.first
}

private func lockCapabilities(_ device: AVCaptureDevice) -> [String: Any] {
    [
        "exposure": device.isExposureModeSupported(.locked),
        "focus": device.isFocusModeSupported(.locked),
        "white_balance": device.isWhiteBalanceModeSupported(.locked),
    ]
}

private func deviceDescription(_ device: AVCaptureDevice) -> [String: Any] {
    [
        "formats": device.formats.map {
            formatDescription($0, activeFormat: device.activeFormat)
        },
        "lock_capabilities": lockCapabilities(device),
        "manufacturer": device.manufacturer,
        "model_id": device.modelID,
        "name": device.localizedName,
        "position": positionName(device.position),
        "unique_id": device.uniqueID,
    ]
}

private func deviceSummary(_ device: AVCaptureDevice) -> [String: Any] {
    var activeFormat = formatDescription(
        device.activeFormat,
        activeFormat: device.activeFormat
    )
    activeFormat.removeValue(forKey: "frame_rate_ranges")
    return [
        "active_format": activeFormat,
        "format_count": device.formats.count,
        "lock_capabilities": lockCapabilities(device),
        "manufacturer": device.manufacturer,
        "model_id": device.modelID,
        "name": device.localizedName,
        "position": positionName(device.position),
        "unique_id": device.uniqueID,
    ]
}

private func positionName(_ position: AVCaptureDevice.Position) -> String {
    switch position {
    case .unspecified:
        return "unspecified"
    case .back:
        return "back"
    case .front:
        return "front"
    @unknown default:
        return "unknown"
    }
}

private func listCommand(_ arguments: ArraySlice<String>) throws -> CommandResult {
    let options = try ParsedOptions(
        arguments: arguments,
        valueNames: [],
        flagNames: ["summary"]
    )
    let authorization = AVCaptureDevice.authorizationStatus(for: .video)
    let devices = videoDevices()
    return CommandResult(successEnvelope(command: "list", fields: [
        "authorization": authorizationName(authorization),
        "devices": (
            options.flags.contains("summary")
            ? devices.map(deviceSummary)
            : devices.map(deviceDescription)
        ),
        "summary": options.flags.contains("summary"),
        "status": "OK",
    ]))
}

private func blockedTCCReason(_ authorization: AVAuthorizationStatus) -> String? {
    switch authorization {
    case .authorized:
        return nil
    case .notDetermined:
        return "BLOCKED_TCC_NOT_DETERMINED"
    case .denied:
        return "BLOCKED_TCC_DENIED"
    case .restricted:
        return "BLOCKED_TCC_RESTRICTED"
    @unknown default:
        return "BLOCKED_TCC_UNKNOWN"
    }
}

private func authorizeCommand(_ arguments: ArraySlice<String>) throws -> CommandResult {
    _ = try ParsedOptions(
        arguments: arguments,
        valueNames: [],
        flagNames: []
    )
    let priorAuthorization = AVCaptureDevice.authorizationStatus(for: .video)
    let completed = DispatchSemaphore(value: 0)
    var requestGranted = false
    AVCaptureDevice.requestAccess(for: .video) { granted in
        requestGranted = granted
        completed.signal()
    }
    let requestCompleted =
        completed.wait(timeout: .now() + authorizationTimeoutSeconds) == .success
    let finalAuthorization = AVCaptureDevice.authorizationStatus(for: .video)
    let granted = requestCompleted
        && requestGranted
        && finalAuthorization == .authorized
    var blockerCodes: [String] = []
    if !requestCompleted {
        blockerCodes.append("BLOCKED_TCC_REQUEST_TIMEOUT")
    } else if let reason = blockedTCCReason(finalAuthorization) {
        blockerCodes.append(reason)
    }

    return CommandResult(
        successEnvelope(command: "authorize", fields: [
            "blocked": !granted,
            "blocker_codes": blockerCodes,
            "final_authorization": authorizationName(finalAuthorization),
            "granted": granted,
            "prior_authorization": authorizationName(priorAuthorization),
            "request_completed": requestCompleted,
            "request_result_granted": requestGranted,
            "status": granted ? "AUTHORIZED" : "BLOCKED",
        ]),
        exitCode: granted ? 0 : 3
    )
}

private func preflightCommand(_ arguments: ArraySlice<String>) throws -> CommandResult {
    let options = try ParsedOptions(
        arguments: arguments,
        valueNames: ["camera-id"],
        flagNames: []
    )
    let cameraID = try options.requireValue("camera-id")
    let authorization = AVCaptureDevice.authorizationStatus(for: .video)
    let device = videoDevices().first { $0.uniqueID == cameraID }
    let selectedFormat = device.flatMap(selectedCameraFormat)
    let capabilities = device.map(lockCapabilities) ?? [
        "exposure": false,
        "focus": false,
        "white_balance": false,
    ]
    var blockers: [String] = []
    if let reason = blockedTCCReason(authorization) {
        blockers.append(reason)
    }
    if device == nil {
        blockers.append("BLOCKED_DEVICE_NOT_FOUND")
    } else if capabilities["exposure"] as? Bool != true ||
              capabilities["white_balance"] as? Bool != true {
        blockers.append("BLOCKED_CAMERA_STABILITY_UNSUPPORTED")
    }

    let fields: [String: Any] = [
        "authorization": authorizationName(authorization),
        "blocked": !blockers.isEmpty,
        "blocker_codes": blockers,
        "camera_id": cameraID,
        "device": device.map(deviceDescription) ?? NSNull(),
        "device_present": device != nil,
        "lock_capabilities": capabilities,
        "selected_format": selectedFormat.map {
            formatDescription($0, activeFormat: device?.activeFormat)
        } ?? NSNull(),
        "selected_frame_rate": selectedFormat.map(preferredFrameRate) ?? NSNull(),
        "selection_policy": "landscape_maximum_resolution_prefer_30_fps",
        "status": blockers.isEmpty ? "READY" : "BLOCKED",
    ]
    return CommandResult(
        successEnvelope(command: "preflight", fields: fields),
        exitCode: blockers.isEmpty ? 0 : 3
    )
}

private func normalizedURL(_ path: String) throws -> URL {
    guard !path.isEmpty else {
        throw ToolError("INVALID_PATH", "A path must not be empty")
    }
    let workingDirectory = URL(
        fileURLWithPath: FileManager.default.currentDirectoryPath,
        isDirectory: true
    )
    return URL(fileURLWithPath: path, relativeTo: workingDirectory).standardizedFileURL
}

private func fileStatus(_ url: URL) -> (exists: Bool, mode: mode_t) {
    var info = stat()
    let result = url.path.withCString { path in
        Darwin.lstat(path, &info)
    }
    return result == 0 ? (true, info.st_mode) : (false, 0)
}

private func fileType(_ mode: mode_t) -> mode_t {
    mode & mode_t(S_IFMT)
}

private func validatePNGInput(_ path: String) throws -> URL {
    let url = try normalizedURL(path)
    let status = fileStatus(url)
    guard status.exists else {
        throw ToolError("INPUT_NOT_FOUND", "Input PNG does not exist", exitCode: 4)
    }
    guard fileType(status.mode) != mode_t(S_IFLNK) else {
        throw ToolError("UNSAFE_INPUT", "Input PNG must not be a symbolic link", exitCode: 4)
    }
    guard fileType(status.mode) == mode_t(S_IFREG) else {
        throw ToolError("UNSAFE_INPUT", "Input PNG must be a regular file", exitCode: 4)
    }
    guard url.pathExtension.lowercased() == "png" else {
        throw ToolError("INVALID_INPUT", "Input must have a .png extension")
    }
    let handle: FileHandle
    do {
        handle = try FileHandle(forReadingFrom: url)
    } catch {
        throw ToolError("INPUT_READ_FAILED", "Could not open input PNG", exitCode: 4)
    }
    defer { try? handle.close() }
    let signature = try handle.read(upToCount: 8) ?? Data()
    guard signature == Data([137, 80, 78, 71, 13, 10, 26, 10]) else {
        throw ToolError("INVALID_INPUT", "Input is not a PNG file")
    }
    return url
}

private func validateNewPNGOutput(_ path: String) throws -> URL {
    let url = try normalizedURL(path)
    guard url.pathExtension.lowercased() == "png" else {
        throw ToolError("INVALID_OUTPUT", "Output must have a .png extension")
    }
    guard !fileStatus(url).exists else {
        throw ToolError(
            "OUTPUT_EXISTS",
            "Output must be a new path and must not already exist",
            exitCode: 4
        )
    }
    let parentStatus = fileStatus(url.deletingLastPathComponent())
    guard parentStatus.exists, fileType(parentStatus.mode) == mode_t(S_IFDIR) else {
        throw ToolError(
            "OUTPUT_DIRECTORY_NOT_FOUND",
            "Output directory does not exist",
            exitCode: 4
        )
    }
    return url
}

private func writeNewFile(_ data: Data, to url: URL) throws {
    let descriptor = url.path.withCString { path in
        Darwin.open(
            path,
            O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW,
            mode_t(S_IRUSR | S_IWUSR)
        )
    }
    guard descriptor >= 0 else {
        throw ToolError(
            "OUTPUT_CREATE_FAILED",
            "Could not create the new output file",
            exitCode: 4
        )
    }

    var complete = false
    defer {
        Darwin.close(descriptor)
        if !complete {
            url.path.withCString { path in
                _ = Darwin.unlink(path)
            }
        }
    }
    guard Darwin.fchmod(descriptor, mode_t(S_IRUSR | S_IWUSR)) == 0 else {
        throw ToolError(
            "OUTPUT_CREATE_FAILED",
            "Could not secure permissions on the new output file",
            exitCode: 4
        )
    }

    do {
        try data.withUnsafeBytes { rawBuffer in
            guard var pointer = rawBuffer.baseAddress else {
                return
            }
            var remaining = rawBuffer.count
            while remaining > 0 {
                let count = Darwin.write(descriptor, pointer, remaining)
                guard count > 0 else {
                    throw ToolError(
                        "OUTPUT_WRITE_FAILED",
                        "Could not write the output file",
                        exitCode: 4
                    )
                }
                remaining -= count
                pointer = pointer.advanced(by: count)
            }
        }
        guard Darwin.fsync(descriptor) == 0 else {
            throw ToolError(
                "OUTPUT_WRITE_FAILED",
                "Could not flush the output file",
                exitCode: 4
            )
        }
        complete = true
    } catch let error as ToolError {
        throw error
    } catch {
        throw ToolError(
            "OUTPUT_WRITE_FAILED",
            "Could not write the output file",
            exitCode: 4
        )
    }
}

private func loadPNG(_ url: URL) throws -> CGImage {
    guard
        let source = CGImageSourceCreateWithURL(url as CFURL, nil),
        CGImageSourceGetCount(source) == 1,
        let image = CGImageSourceCreateImageAtIndex(
            source,
            0,
            [kCGImageSourceShouldCache: true] as CFDictionary
        )
    else {
        throw ToolError(
            "PNG_DECODE_FAILED",
            "Input PNG could not be decoded as a single image",
            exitCode: 5
        )
    }
    return image
}

private func encodePNG(_ image: CGImage) throws -> Data {
    let data = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(
        data,
        "public.png" as CFString,
        1,
        nil
    ) else {
        throw ToolError("PNG_ENCODE_FAILED", "Could not create PNG encoder", exitCode: 5)
    }
    CGImageDestinationAddImage(destination, image, [
        kCGImagePropertyHasAlpha: image.alphaInfo != .none,
    ] as CFDictionary)
    guard CGImageDestinationFinalize(destination) else {
        throw ToolError("PNG_ENCODE_FAILED", "Could not encode PNG", exitCode: 5)
    }
    return data as Data
}

private func sha256(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}

private func iso8601(_ date: Date) -> String {
    let formatter = ISO8601DateFormatter()
    formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    formatter.timeZone = TimeZone(secondsFromGMT: 0)
    return formatter.string(from: date)
}

private func pinVisionRequestToCPU(_ request: VNRequest) throws {
    let devicesByStage: [VNComputeStage: [MLComputeDevice]]
    do {
        devicesByStage = try request.supportedComputeStageDevices
    } catch {
        throw ToolError(
            "VISION_CONFIGURATION_FAILED",
            "Could not query deterministic Vision compute devices",
            exitCode: 5
        )
    }
    guard !devicesByStage.isEmpty else {
        throw ToolError(
            "VISION_CONFIGURATION_FAILED",
            "Vision reported no supported compute stages",
            exitCode: 5
        )
    }
    for (stage, devices) in devicesByStage {
        guard let cpu = devices.first(where: { device in
            if case .cpu = device {
                return true
            }
            return false
        }) else {
            throw ToolError(
                "VISION_CPU_UNAVAILABLE",
                "A Vision request stage does not support deterministic CPU execution",
                exitCode: 5
            )
        }
        request.setComputeDevice(cpu, for: stage)
    }
}

private func lockCurrentCameraState(_ device: AVCaptureDevice) throws -> [String: Any] {
    let capabilities = lockCapabilities(device)
    var locked: [String: Bool] = [
        "exposure": false,
        "focus": false,
        "white_balance": false,
    ]
    do {
        try device.lockForConfiguration()
    } catch {
        throw ToolError(
            "CAMERA_CONFIGURATION_FAILED",
            "Could not lock camera configuration",
            exitCode: 5
        )
    }
    defer { device.unlockForConfiguration() }

    if device.isFocusModeSupported(.locked) {
        device.focusMode = .locked
        locked["focus"] = true
    }
    if device.isExposureModeSupported(.locked) {
        device.exposureMode = .locked
        locked["exposure"] = true
    }
    if device.isWhiteBalanceModeSupported(.locked) {
        device.whiteBalanceMode = .locked
        locked["white_balance"] = true
    }
    return [
        "capabilities": capabilities,
        "locked": locked,
    ]
}

private func waitForCameraSettle(_ device: AVCaptureDevice) throws -> [String: Any] {
    let startedAt = Date()
    let capabilities = lockCapabilities(device)
    while true {
        let focusAdjusting =
            (capabilities["focus"] as? Bool == true) && device.isAdjustingFocus
        let exposureAdjusting =
            (capabilities["exposure"] as? Bool == true) && device.isAdjustingExposure
        let whiteBalanceAdjusting =
            (capabilities["white_balance"] as? Bool == true) && device.isAdjustingWhiteBalance
        let elapsed = Date().timeIntervalSince(startedAt)
        if !focusAdjusting && !exposureAdjusting && !whiteBalanceAdjusting {
            return [
                "settled": true,
                "waited_ms": Int((elapsed * 1_000).rounded()),
            ]
        }
        guard elapsed < cameraSettleTimeoutSeconds else {
            throw ToolError(
                "CAMERA_SETTLE_TIMEOUT",
                "Camera controls did not settle within the bounded timeout",
                exitCode: 5,
                details: [
                    "exposure_adjusting": exposureAdjusting,
                    "focus_adjusting": focusAdjusting,
                    "timeout_ms": Int(cameraSettleTimeoutSeconds * 1_000),
                    "white_balance_adjusting": whiteBalanceAdjusting,
                ]
            )
        }
        Thread.sleep(forTimeInterval: 0.025)
    }
}

private func configureSelectedFormat(
    _ format: AVCaptureDevice.Format,
    on device: AVCaptureDevice
) throws -> Double {
    guard let frameDuration = preferredFrameDuration(format),
          frameDuration.isValid,
          frameDuration.isNumeric,
          frameDuration.value > 0
    else {
        throw ToolError(
            "CAMERA_FORMAT_UNSUPPORTED",
            "Selected camera format has no usable frame rate",
            exitCode: 5
        )
    }
    do {
        try device.lockForConfiguration()
    } catch {
        throw ToolError(
            "CAMERA_CONFIGURATION_FAILED",
            "Could not configure the selected camera format",
            exitCode: 5
        )
    }
    defer { device.unlockForConfiguration() }
    device.activeFormat = format
    device.activeVideoMinFrameDuration = frameDuration
    device.activeVideoMaxFrameDuration = frameDuration
    return 1.0 / CMTimeGetSeconds(frameDuration)
}

private func captureFrame(
    device: AVCaptureDevice,
    selectedFormat: AVCaptureDevice.Format,
    warmupMilliseconds: Int
) throws -> (
    data: Data,
    width: Int,
    height: Int,
    capturedAt: Date,
    presentationTime: Double,
    burstCount: Int,
    lockState: [String: Any],
    selectedFormat: [String: Any],
    selectedFrameRate: Double
) {
    let session = AVCaptureSession()
    let collector = FrameCollector()
    let callbackQueue = DispatchQueue(label: "com.johnkozaris.espBobeToy.bobe-capture.frames")
    let output = AVCaptureVideoDataOutput()
    let selectedFrameRate = try configureSelectedFormat(selectedFormat, on: device)

    session.beginConfiguration()
    defer {
        if session.isRunning {
            session.stopRunning()
        }
    }
    let input: AVCaptureDeviceInput
    do {
        input = try AVCaptureDeviceInput(device: device)
    } catch {
        session.commitConfiguration()
        throw ToolError(
            "CAMERA_OPEN_FAILED",
            "Could not create an input for the selected camera",
            exitCode: 5
        )
    }
    guard session.canAddInput(input) else {
        session.commitConfiguration()
        throw ToolError(
            "CAMERA_OPEN_FAILED",
            "Selected camera cannot be added to a capture session",
            exitCode: 5
        )
    }
    session.addInput(input)
    output.videoSettings = [
        kCVPixelBufferPixelFormatTypeKey as String: Int(kCVPixelFormatType_32BGRA),
    ]
    output.alwaysDiscardsLateVideoFrames = false
    output.setSampleBufferDelegate(collector, queue: callbackQueue)
    guard session.canAddOutput(output) else {
        session.commitConfiguration()
        throw ToolError(
            "CAMERA_OPEN_FAILED",
            "Video output cannot be added to the capture session",
            exitCode: 5
        )
    }
    session.addOutput(output)
    session.commitConfiguration()

    let sessionQueue = DispatchQueue(label: "com.johnkozaris.espBobeToy.bobe-capture.session")
    let started = DispatchSemaphore(value: 0)
    sessionQueue.async {
        session.startRunning()
        started.signal()
    }
    guard started.wait(timeout: .now() + captureStartTimeoutSeconds) == .success else {
        sessionQueue.async {
            if session.isRunning {
                session.stopRunning()
            }
        }
        throw ToolError(
            "CAPTURE_TIMEOUT",
            "Camera session did not start within the bounded timeout",
            exitCode: 5
        )
    }
    guard session.isRunning else {
        throw ToolError(
            "CAMERA_OPEN_FAILED",
            "Camera session did not start",
            exitCode: 5
        )
    }

    if warmupMilliseconds > 0 {
        Thread.sleep(forTimeInterval: Double(warmupMilliseconds) / 1_000.0)
    }
    let settleState = try waitForCameraSettle(device)
    var lockState = try lockCurrentCameraState(device)
    lockState["settle"] = settleState
    collector.beginBurst()
    let frames = collector.waitForBurst(
        count: captureBurstSize,
        timeout: captureBurstTimeoutSeconds
    )
    guard let frame = frames.last, frames.count == captureBurstSize else {
        throw ToolError(
            "CAPTURE_TIMEOUT",
            "A complete frame burst was not captured within the bounded timeout",
            exitCode: 5,
            details: [
                "captured_frames": frames.count,
                "required_frames": captureBurstSize,
            ]
        )
    }
    guard let pixelBuffer = CMSampleBufferGetImageBuffer(frame.sampleBuffer) else {
        throw ToolError("CAPTURE_FAILED", "Captured frame has no image buffer", exitCode: 5)
    }

    let ciImage = CIImage(cvPixelBuffer: pixelBuffer)
    let context = CIContext(options: [
        .cacheIntermediates: false,
        .useSoftwareRenderer: true,
    ])
    guard let cgImage = context.createCGImage(ciImage, from: ciImage.extent) else {
        throw ToolError("CAPTURE_FAILED", "Could not render captured frame", exitCode: 5)
    }
    let data = try encodePNG(cgImage)
    let presentationTime = CMSampleBufferGetPresentationTimeStamp(frame.sampleBuffer)
    return (
        data,
        cgImage.width,
        cgImage.height,
        frame.capturedAt,
        presentationTime.isValid ? CMTimeGetSeconds(presentationTime) : -1,
        frames.count,
        lockState,
        formatDescription(device.activeFormat, activeFormat: device.activeFormat),
        selectedFrameRate
    )
}

private func frameCommand(_ arguments: ArraySlice<String>) throws -> CommandResult {
    let options = try ParsedOptions(
        arguments: arguments,
        valueNames: ["camera-id", "output", "warmup-ms"],
        flagNames: []
    )
    let cameraID = try options.requireValue("camera-id")
    let outputURL = try validateNewPNGOutput(try options.requireValue("output"))
    let warmupText = options.values["warmup-ms"] ?? "1500"
    guard
        let warmupMilliseconds = Int(warmupText),
        (minimumWarmupMilliseconds...maximumWarmupMilliseconds).contains(warmupMilliseconds)
    else {
        throw ToolError(
            "INVALID_ARGUMENT",
            "--warmup-ms must be an integer from \(minimumWarmupMilliseconds) through \(maximumWarmupMilliseconds)"
        )
    }

    let authorization = AVCaptureDevice.authorizationStatus(for: .video)
    guard authorization == .authorized else {
        throw ToolError(
            blockedTCCReason(authorization) ?? "BLOCKED_TCC",
            "Camera access is not already authorized; no permission prompt was attempted",
            exitCode: 3,
            details: ["authorization": authorizationName(authorization)]
        )
    }
    guard let device = videoDevices().first(where: { $0.uniqueID == cameraID }) else {
        throw ToolError(
            "DEVICE_NOT_FOUND",
            "No camera exactly matches --camera-id",
            exitCode: 3,
            details: ["camera_id": cameraID]
        )
    }
    guard let selectedFormat = selectedCameraFormat(device) else {
        throw ToolError(
            "CAMERA_FORMAT_UNAVAILABLE",
            "Selected camera reports no capture formats",
            exitCode: 5
        )
    }

    let startedAt = Date()
    let capture = try captureFrame(
        device: device,
        selectedFormat: selectedFormat,
        warmupMilliseconds: warmupMilliseconds
    )
    try writeNewFile(capture.data, to: outputURL)
    return CommandResult(successEnvelope(command: "frame", fields: [
        "burst_frame_count": capture.burstCount,
        "camera_id": cameraID,
        "capture_started_at": iso8601(startedAt),
        "captured_at": iso8601(capture.capturedAt),
        "height": capture.height,
        "lock_state": capture.lockState,
        "output": outputURL.path,
        "presentation_time_seconds": capture.presentationTime,
        "selected_format": capture.selectedFormat,
        "selected_frame_rate": capture.selectedFrameRate,
        "selection_policy": "landscape_maximum_resolution_prefer_30_fps",
        "sha256": sha256(capture.data),
        "warmup_ms": warmupMilliseconds,
        "width": capture.width,
    ]))
}

private func ocrCommand(_ arguments: ArraySlice<String>) throws -> CommandResult {
    let options = try ParsedOptions(
        arguments: arguments,
        valueNames: ["input"],
        flagNames: []
    )
    let inputURL = try validatePNGInput(try options.requireValue("input"))
    let image = try loadPNG(inputURL)

    let request = VNRecognizeTextRequest()
    request.revision = textRecognitionRevision
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = false
    request.recognitionLanguages = ["en-US"]
    request.minimumTextHeight = 0.01
    try pinVisionRequestToCPU(request)
    let handler = VNImageRequestHandler(cgImage: image, orientation: .up, options: [:])
    do {
        try handler.perform([request])
    } catch {
        throw ToolError(
            "OCR_FAILED",
            "Vision text recognition failed",
            exitCode: 5,
            details: ["vision_error": String(describing: error)]
        )
    }

    let observations = (request.results ?? []).compactMap {
        observation -> (VNRecognizedTextObservation, VNRecognizedText)? in
        guard let candidate = observation.topCandidates(1).first else {
            return nil
        }
        return (observation, candidate)
    }.sorted { first, second in
        let verticalDifference = first.0.boundingBox.maxY - second.0.boundingBox.maxY
        if abs(verticalDifference) > 0.001 {
            return verticalDifference > 0
        }
        return first.0.boundingBox.minX < second.0.boundingBox.minX
    }

    let results = observations.map { observation, candidate in
        let box = observation.boundingBox
        return [
            "bounding_box": [
                "height": box.height,
                "width": box.width,
                "x": box.minX,
                "y": box.minY,
            ],
            "confidence": Double(candidate.confidence),
            "text": candidate.string,
        ] as [String: Any]
    }
    return CommandResult(successEnvelope(command: "ocr", fields: [
        "height": image.height,
        "input": inputURL.path,
        "compute_device": "cpu",
        "recognition_languages": ["en-US"],
        "recognition_level": "accurate",
        "request_revision": textRecognitionRevision,
        "results": results,
        "uses_language_correction": false,
        "width": image.width,
    ]))
}

private func vector(_ point: CGPoint, in extent: CGRect) -> CIVector {
    CIVector(
        x: extent.minX + point.x * extent.width,
        y: extent.minY + point.y * extent.height
    )
}

private func pointJSON(_ point: CGPoint) -> [String: Any] {
    ["x": point.x, "y": point.y]
}

private func rectifyCommand(_ arguments: ArraySlice<String>) throws -> CommandResult {
    let options = try ParsedOptions(
        arguments: arguments,
        valueNames: ["height", "input", "output", "width"],
        flagNames: []
    )
    let inputURL = try validatePNGInput(try options.requireValue("input"))
    let outputURL = try validateNewPNGOutput(try options.requireValue("output"))
    let widthText = try options.requireValue("width")
    let heightText = try options.requireValue("height")
    guard
        let width = Int(widthText),
        let height = Int(heightText),
        (1...4096).contains(width),
        (1...4096).contains(height)
    else {
        throw ToolError(
            "INVALID_ARGUMENT",
            "--width and --height must be integers from 1 through 4096"
        )
    }
    let image = try loadPNG(inputURL)

    let request = VNDetectRectanglesRequest()
    request.revision = rectangleRecognitionRevision
    request.maximumObservations = 4
    request.minimumConfidence = 0.8
    request.minimumAspectRatio = 0.1
    request.maximumAspectRatio = 1.0
    request.minimumSize = 0.15
    request.quadratureTolerance = 20
    try pinVisionRequestToCPU(request)
    let handler = VNImageRequestHandler(cgImage: image, orientation: .up, options: [:])
    do {
        try handler.perform([request])
    } catch {
        throw ToolError(
            "RECTANGLE_DETECTION_FAILED",
            "Vision rectangle detection failed",
            exitCode: 5,
            details: ["vision_error": String(describing: error)]
        )
    }
    let rectangles = (request.results ?? []).sorted { $0.confidence > $1.confidence }
    guard let rectangle = rectangles.first, rectangle.confidence >= 0.8 else {
        throw ToolError(
            "NO_HIGH_CONFIDENCE_RECTANGLE",
            "No rectangle met the pinned confidence threshold",
            exitCode: 5,
            details: ["minimum_confidence": 0.8]
        )
    }

    let ciImage = CIImage(cgImage: image)
    guard let filter = CIFilter(name: "CIPerspectiveCorrection") else {
        throw ToolError(
            "RECTIFICATION_FAILED",
            "Core Image perspective correction is unavailable",
            exitCode: 5
        )
    }
    filter.setValue(ciImage, forKey: kCIInputImageKey)
    filter.setValue(vector(rectangle.topLeft, in: ciImage.extent), forKey: "inputTopLeft")
    filter.setValue(vector(rectangle.topRight, in: ciImage.extent), forKey: "inputTopRight")
    filter.setValue(vector(rectangle.bottomLeft, in: ciImage.extent), forKey: "inputBottomLeft")
    filter.setValue(vector(rectangle.bottomRight, in: ciImage.extent), forKey: "inputBottomRight")
    guard let corrected = filter.outputImage, corrected.extent.width > 0, corrected.extent.height > 0 else {
        throw ToolError(
            "RECTIFICATION_FAILED",
            "Core Image could not rectify the detected rectangle",
            exitCode: 5
        )
    }

    let originAdjusted = corrected.transformed(
        by: CGAffineTransform(
            translationX: -corrected.extent.minX,
            y: -corrected.extent.minY
        )
    )
    let scaled = originAdjusted.transformed(
        by: CGAffineTransform(
            scaleX: CGFloat(width) / originAdjusted.extent.width,
            y: CGFloat(height) / originAdjusted.extent.height
        )
    )
    let outputExtent = CGRect(x: 0, y: 0, width: width, height: height)
    let context = CIContext(options: [
        .cacheIntermediates: false,
        .useSoftwareRenderer: true,
    ])
    guard let outputImage = context.createCGImage(scaled, from: outputExtent) else {
        throw ToolError(
            "RECTIFICATION_FAILED",
            "Could not render rectified image",
            exitCode: 5
        )
    }
    let data = try encodePNG(outputImage)
    try writeNewFile(data, to: outputURL)
    return CommandResult(successEnvelope(command: "rectify", fields: [
        "detected_rectangle": [
            "bottom_left": pointJSON(rectangle.bottomLeft),
            "bottom_right": pointJSON(rectangle.bottomRight),
            "confidence": Double(rectangle.confidence),
            "top_left": pointJSON(rectangle.topLeft),
            "top_right": pointJSON(rectangle.topRight),
        ],
        "height": height,
        "compute_device": "cpu",
        "input": inputURL.path,
        "output": outputURL.path,
        "request_revision": rectangleRecognitionRevision,
        "sha256": sha256(data),
        "width": width,
    ]))
}

private let rootUsage = """
Usage: bobe-capture COMMAND [options]

Invoke through tools/bobe_capture.sh. That wrapper builds/signs the app whose
bundle identifier owns macOS TCC permission; do not call the ignored .tmp binary.

Commands:
  list        Enumerate cameras and persistent unique IDs; never prompts.
  preflight   Check TCC and deterministic controls for one --camera-id; never prompts.
  authorize   Request the user-visible macOS camera consent dialog.
  frame       Capture from one --camera-id to a new PNG path.
  ocr         Run Vision OCR on an existing image; no camera access.
  rectify     Perspective-correct an existing image; no camera access.

All commands emit one JSON object.
Use "bobe-capture COMMAND --help" for options.
Exit codes: 0 success/ready, 2 invalid arguments, 3 blocked, 4 file/path error,
5 unavailable operation/capture, 70 unexpected internal failure.
"""

private func commandUsage(_ command: String) -> String? {
    switch command {
    case "list":
        return "Usage: bobe-capture list [--summary]"
    case "authorize":
        return "Usage: bobe-capture authorize"
    case "preflight":
        return "Usage: bobe-capture preflight --camera-id ID"
    case "frame":
        return """
        Usage: bobe-capture frame --camera-id ID --output NEW.png \
        [--warmup-ms 250..10000]
        """
    case "ocr":
        return "Usage: bobe-capture ocr --input IMAGE"
    case "rectify":
        return """
        Usage: bobe-capture rectify --input IMAGE --output NEW.png \
        --width 1..4096 --height 1..4096
        """
    default:
        return nil
    }
}

private func run(arguments: [String]) throws -> CommandResult {
    guard arguments.count >= 2 else {
        throw ToolError(
            "INVALID_ARGUMENT",
            "Expected a command: list, authorize, preflight, frame, ocr, or rectify"
        )
    }
    if ["--help", "-h", "help"].contains(arguments[1]) {
        guard arguments.count == 2 else {
            throw ToolError("INVALID_ARGUMENT", "--help does not accept arguments")
        }
        print(rootUsage)
        return CommandResult([:])
    }
    if arguments[1] == "--version" {
        guard arguments.count == 2 else {
            throw ToolError("INVALID_ARGUMENT", "--version does not accept arguments")
        }
        print("bobe-capture \(toolVersion)")
        return CommandResult([:])
    }
    let command = arguments[1]
    let commandArguments = arguments.dropFirst(2)
    if commandArguments.count == 1,
       let argument = commandArguments.first,
       ["--help", "-h"].contains(argument) {
        guard let usage = commandUsage(command) else {
            throw ToolError("INVALID_ARGUMENT", "Unknown command: \(command)")
        }
        print(usage)
        return CommandResult([:])
    }
    switch command {
    case "list":
        return try listCommand(commandArguments)
    case "authorize":
        return try authorizeCommand(commandArguments)
    case "preflight":
        return try preflightCommand(commandArguments)
    case "frame":
        return try frameCommand(commandArguments)
    case "ocr":
        return try ocrCommand(commandArguments)
    case "rectify":
        return try rectifyCommand(commandArguments)
    default:
        throw ToolError("INVALID_ARGUMENT", "Unknown command: \(command)")
    }
}

do {
    let result = try run(arguments: CommandLine.arguments)
    if !result.value.isEmpty {
        writeJSON(result.value, to: .standardOutput)
    }
    exit(result.exitCode)
} catch let error as ToolError {
    writeJSON(errorEnvelope(error), to: .standardError)
    exit(error.exitCode)
} catch {
    let wrapped = ToolError(
        "INTERNAL_ERROR",
        "Unexpected failure",
        exitCode: 70,
        details: ["underlying_error": String(describing: error)]
    )
    writeJSON(errorEnvelope(wrapped), to: .standardError)
    exit(wrapped.exitCode)
}
