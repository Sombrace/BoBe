import Darwin
import Foundation

nonisolated enum WB12OnboardingError: LocalizedError {
    case invalidSSID
    case invalidPassword
    case deviceNotFound
    case ambiguousDevices
    case deviceRejected(String)
    case timedOut

    var errorDescription: String? {
        switch self {
        case .invalidSSID: L10n.tr("setup.physical.error.name_too_long")
        case .invalidPassword: L10n.tr("setup.physical.error.password_too_long")
        case .deviceNotFound: L10n.tr("setup.physical.error.not_found")
        case .ambiguousDevices: L10n.tr("setup.physical.error.several")
        case let .deviceRejected(code):
            switch code {
            case "AUTH": L10n.tr("setup.physical.error.wrong_password")
            case "NO_AP": L10n.tr("setup.physical.error.no_network")
            case "BUSY", "NOT_READY": L10n.tr("setup.physical.error.busy")
            case "TIMEOUT": L10n.tr("setup.physical.error.join_timeout")
            default: L10n.tr("setup.physical.error.rejected")
            }
        case .timedOut: L10n.tr("setup.physical.error.timed_out")
        }
    }
}

nonisolated struct WB12OnboardingProtocol {
    static let prefix = "@BOBE-WIFI/1 "

    static func base64URL(_ value: String) -> String {
        Data(value.utf8)
            .base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    static func info(nonce: String) -> Data {
        Data("\(self.prefix)INFO \(nonce)\n".utf8)
    }

    static func configure(nonce: String, ssid: String, password: String) -> Data {
        let encodedPassword = password.isEmpty ? "-" : self.base64URL(password)
        return Data(
            "\(self.prefix)SET \(nonce) \(self.base64URL(ssid)) \(encodedPassword)\n".utf8
        )
    }

    static func identity(from line: String, nonce: String) -> String? {
        let fields = line.split(separator: " ")
        guard fields.count == 5,
              fields[0] == "@BOBE-WIFI/1",
              fields[1] == "ID",
              fields[2] == Substring(nonce),
              fields[3].count == 12,
              fields[4] == "WB-12"
        else { return nil }
        return String(fields[3])
    }
}

actor WB12OnboardingService {
    static let shared = WB12OnboardingService()

    private struct Candidate {
        let fileDescriptor: Int32
        let identity: String
    }

    func configure(ssid: String, password: String) async throws -> String {
        guard (1 ... 32).contains(ssid.utf8.count) else {
            throw WB12OnboardingError.invalidSSID
        }
        guard password.isEmpty || (8 ... 63).contains(password.utf8.count) else {
            throw WB12OnboardingError.invalidPassword
        }
        let paths = try FileManager.default
            .contentsOfDirectory(atPath: "/dev")
            .filter { $0.hasPrefix("cu.usbmodem") }
            .sorted()
            .prefix(8)
            .map { "/dev/\($0)" }
        var matches: [Candidate] = []
        defer {
            for candidate in matches {
                Darwin.close(candidate.fileDescriptor)
            }
        }
        for path in paths {
            do {
                if let candidate = try await self.identify(path: path) {
                    matches.append(candidate)
                }
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                continue
            }
        }
        guard matches.count == 1 else {
            throw matches.isEmpty
                ? WB12OnboardingError.deviceNotFound
                : WB12OnboardingError.ambiguousDevices
        }
        let candidate = matches[0]
        let nonce = Self.nonce()
        try Self.writeAll(
            WB12OnboardingProtocol.configure(
                nonce: nonce,
                ssid: ssid,
                password: password
            ),
            to: candidate.fileDescriptor
        )
        let deadline = ContinuousClock.now.advanced(by: .seconds(45))
        var pending = Data()
        while ContinuousClock.now < deadline {
            try Task.checkCancellation()
            let result = try await Self.readProtocolLine(
                from: candidate.fileDescriptor,
                deadline: deadline,
                pending: pending
            )
            pending = result.remaining
            if let line = result.line {
                if line == "\(WB12OnboardingProtocol.prefix)ACK \(nonce)" {
                    continue
                } else if line == "\(WB12OnboardingProtocol.prefix)OK \(nonce)" {
                    return candidate.identity
                } else if line.hasPrefix(
                    "\(WB12OnboardingProtocol.prefix)ERR \(nonce) "
                ) {
                    throw WB12OnboardingError.deviceRejected(
                        String(line.split(separator: " ").last ?? "UNKNOWN")
                    )
                }
            }
        }
        throw WB12OnboardingError.timedOut
    }

    private func identify(path: String) async throws -> Candidate? {
        let descriptor = Darwin.open(
            path,
            O_RDWR | O_NOCTTY | O_NONBLOCK | O_CLOEXEC | O_NOFOLLOW
        )
        guard descriptor >= 0 else { return nil }
        guard ioctl(descriptor, TIOCEXCL) == 0 else {
            Darwin.close(descriptor)
            return nil
        }
        let nonce = Self.nonce()
        do {
            try Self.writeAll(
                WB12OnboardingProtocol.info(nonce: nonce),
                to: descriptor
            )
            let deadline = ContinuousClock.now.advanced(by: .seconds(2))
            var pending = Data()
            while ContinuousClock.now < deadline {
                try Task.checkCancellation()
                let result = try await Self.readProtocolLine(
                    from: descriptor,
                    deadline: deadline,
                    pending: pending
                )
                pending = result.remaining
                if let line = result.line,
                   let identity = WB12OnboardingProtocol.identity(
                    from: line,
                    nonce: nonce
                ) {
                    return Candidate(
                        fileDescriptor: descriptor,
                        identity: identity
                    )
                }
            }
        } catch {
            Darwin.close(descriptor)
            throw error
        }
        Darwin.close(descriptor)
        return nil
    }

    private static func nonce() -> String {
        var bytes = [UInt8](repeating: 0, count: 16)
        arc4random_buf(&bytes, bytes.count)
        return bytes.map { String(format: "%02x", $0) }.joined()
    }

    private static func writeAll(_ data: Data, to descriptor: Int32) throws {
        try data.withUnsafeBytes { bytes in
            guard let base = bytes.baseAddress else { return }
            var offset = 0
            while offset < bytes.count {
                let count = Darwin.write(
                    descriptor,
                    base.advanced(by: offset),
                    bytes.count - offset
                )
                if count > 0 {
                    offset += count
                } else if errno == EAGAIN || errno == EINTR {
                    usleep(10_000)
                } else {
                    throw POSIXError(.EIO)
                }
            }
        }
    }

    private static func readProtocolLine(
        from descriptor: Int32,
        deadline: ContinuousClock.Instant,
        pending initial: Data
    ) async throws -> (line: String?, remaining: Data) {
        var pending = initial
        var bytes = [UInt8](repeating: 0, count: 256)
        while ContinuousClock.now < deadline {
            try Task.checkCancellation()
            if let line = Self.extractProtocolLine(from: &pending) {
                return (line, pending)
            }
            let count = Darwin.read(descriptor, &bytes, bytes.count)
            if count > 0 {
                pending.append(contentsOf: bytes[..<count])
                if let line = Self.extractProtocolLine(from: &pending) {
                    return (line, pending)
                }
                if pending.count > 4_096 { pending.removeAll(keepingCapacity: true) }
            } else if count < 0, errno != EAGAIN, errno != EINTR {
                throw POSIXError(.EIO)
            }
            try await Task.sleep(for: .milliseconds(25))
        }
        return (nil, pending)
    }

    private static func extractProtocolLine(from pending: inout Data) -> String? {
        while let newline = pending.firstIndex(of: 0x0A) {
            let lineData = pending[..<newline]
            pending.removeSubrange(...newline)
            if let line = String(data: lineData, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines),
               line.hasPrefix(WB12OnboardingProtocol.prefix) {
                return line
            }
        }
        return nil
    }
}
