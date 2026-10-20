import AppKit
import CoreGraphics
import ScreenCaptureKit
import Vision

nonisolated enum ScreenReader {
    static let maxCharacters = 6_000
    static let longestSide = 3_200
    static let permissionMissing = "permission_missing"
    static let screenLocked = "screen_locked"

    private static let rowHeight = 0.008
    private static let minimumConfidence: Float = 0.3

    struct Line: Sendable, Equatable {
        let text: String
        let top: Double
        let left: Double
    }

    private struct Frontmost: Sendable {
        let name: String?
        let bundleIdentifier: String?
    }

    @concurrent
    static func read() async -> BodyReply {
        guard CGPreflightScreenCaptureAccess() else { return .declined(self.permissionMissing) }
        guard !self.screenIsLocked() else { return .declined(self.screenLocked) }
        let frontmost = await MainActor.run {
            let app = NSWorkspace.shared.frontmostApplication
            return Frontmost(
                name: app?.localizedName.flatMap { $0.isEmpty ? nil : $0 },
                bundleIdentifier: app?.bundleIdentifier
            )
        }
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
            guard let display = content.displays.first(where: { $0.displayID == CGMainDisplayID() })
                ?? content.displays.first
            else { return .failed("no_display") }
            let ownBundle = Bundle.main.bundleIdentifier
            let ownWindows = content.windows.filter { $0.owningApplication?.bundleIdentifier == ownBundle }
            let filter = SCContentFilter(display: display, excludingWindows: ownWindows)
            let configuration = SCStreamConfiguration()
            let size = self.boundedSize(for: display.displayID)
            configuration.width = size.width
            configuration.height = size.height
            configuration.showsCursor = true
            let image = try await SCScreenshotManager.captureImage(
                contentFilter: filter,
                configuration: configuration
            )
            let lines = try await self.recognizeText(in: image)
            return .completed(ScreenReadingWire(
                text: self.arrange(lines),
                application: frontmost.name,
                windowTitle: self.windowTitle(in: content.windows, ownedBy: frontmost.bundleIdentifier)
            ))
        } catch {
            return .failed(String(error.localizedDescription.prefix(200)))
        }
    }

    static func arrange(_ lines: [Line]) -> String {
        let ordered = lines
            .filter { !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
            .sorted { lhs, rhs in
                let lhsRow = self.row(lhs.top)
                let rhsRow = self.row(rhs.top)
                return lhsRow == rhsRow ? lhs.left < rhs.left : lhsRow < rhsRow
            }
        var text = ""
        var count = 0
        for line in ordered {
            let piece = line.text.trimmingCharacters(in: .whitespacesAndNewlines)
            let pieceCount = piece.unicodeScalars.count
            if count + pieceCount + 1 > self.maxCharacters { break }
            if !text.isEmpty {
                text.append("\n")
                count += 1
            }
            text.append(piece)
            count += pieceCount
        }
        return text
    }

    static func boundedSize(width: Int, height: Int) -> (width: Int, height: Int) {
        let width = max(width, 1)
        let height = max(height, 1)
        let longest = max(width, height)
        let bounded = min(longest, self.longestSide)
        return (max(width * bounded / longest, 1), max(height * bounded / longest, 1))
    }

    private static func boundedSize(for displayID: CGDirectDisplayID) -> (width: Int, height: Int) {
        if let mode = CGDisplayCopyDisplayMode(displayID) {
            return self.boundedSize(width: mode.pixelWidth, height: mode.pixelHeight)
        }
        return self.boundedSize(width: CGDisplayPixelsWide(displayID), height: CGDisplayPixelsHigh(displayID))
    }

    private static func row(_ top: Double) -> Int {
        Int((top / self.rowHeight).rounded())
    }

    static func recognizeText(in image: CGImage) async throws -> [Line] {
        var request = RecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = true
        request.automaticallyDetectsLanguage = true
        return try await request.perform(on: image).compactMap { observation in
            guard let candidate = observation.topCandidates(1).first,
                  candidate.confidence >= self.minimumConfidence
            else { return nil }
            let box = observation.boundingBox
            return Line(text: candidate.string, top: 1 - (box.origin.y + box.height), left: box.origin.x)
        }
    }

    private static func windowTitle(in windows: [SCWindow], ownedBy bundleIdentifier: String?) -> String? {
        guard let bundleIdentifier else { return nil }
        return windows
            .filter { $0.windowLayer == 0 && $0.isOnScreen && $0.owningApplication?.bundleIdentifier == bundleIdentifier }
            .compactMap { $0.title?.trimmingCharacters(in: .whitespacesAndNewlines) }
            .first { !$0.isEmpty }
    }

    private static func screenIsLocked() -> Bool {
        guard let session = CGSessionCopyCurrentDictionary() as? [String: Any] else { return false }
        return session["CGSSessionScreenIsLocked"] as? Bool ?? false
    }
}
