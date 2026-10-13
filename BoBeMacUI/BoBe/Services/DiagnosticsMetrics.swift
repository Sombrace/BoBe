import Foundation
import MetricKit
import OSLog

private let diagnosticsLogger = Logger(subsystem: "com.bobe.app", category: "Diagnostics")

struct DiagnosticsSummary: Codable, Equatable, Sendable {
    let periodStart: Date
    let periodEnd: Date
    let applicationVersion: String?
    let includesMultipleApplicationVersions: Bool
    let cpuSeconds: Double?
    let gpuSeconds: Double?
    let foregroundSeconds: Double?
    let backgroundSeconds: Double?
    let peakMemoryBytes: Double?
    let logicalWriteBytes: Double?
    let wifiUploadBytes: Double?
    let wifiDownloadBytes: Double?
    let cellularUploadBytes: Double?
    let cellularDownloadBytes: Double?
    let cpuExceptionCount: Int
    let diskWriteExceptionCount: Int
    let hangCount: Int
    let crashCount: Int
}

struct DiagnosticsArchive: Codable, Equatable, Sendable {
    static let schemaVersion = 1

    let schemaVersion: Int
    let summaries: [DiagnosticsSummary]

    init(summaries: [DiagnosticsSummary]) {
        self.schemaVersion = Self.schemaVersion
        self.summaries = summaries
    }
}

struct DiagnosticsStore {
    static let maximumRecords = 30
    static let maximumAge: TimeInterval = 30 * 24 * 60 * 60

    let fileURL: URL
    private let fileManager: FileManager

    init(fileURL: URL, fileManager: FileManager = .default) {
        self.fileURL = fileURL
        self.fileManager = fileManager
    }

    func load(now: Date = .now) throws -> DiagnosticsArchive {
        guard self.fileManager.fileExists(atPath: self.fileURL.path) else {
            return DiagnosticsArchive(summaries: [])
        }
        let data = try Data(contentsOf: self.fileURL)
        let archive = try Self.decoder.decode(DiagnosticsArchive.self, from: data)
        return DiagnosticsArchive(summaries: Self.retained(archive.summaries, now: now))
    }

    func append(_ summaries: [DiagnosticsSummary], now: Date = .now) throws {
        let existing = try self.load(now: now).summaries
        try self.write(DiagnosticsArchive(summaries: Self.retained(existing + summaries, now: now)))
    }

    func export(to destination: URL, now: Date = .now) throws {
        let data = try Self.encoder.encode(try self.load(now: now))
        try data.write(to: destination, options: .atomic)
    }

    func delete() throws {
        guard self.fileManager.fileExists(atPath: self.fileURL.path) else { return }
        try self.fileManager.removeItem(at: self.fileURL)
    }

    static func retained(_ summaries: [DiagnosticsSummary], now: Date) -> [DiagnosticsSummary] {
        let cutoff = now.addingTimeInterval(-Self.maximumAge)
        return Array(
            summaries
                .filter { $0.periodEnd >= cutoff && $0.periodEnd <= now }
                .sorted { $0.periodEnd < $1.periodEnd }
                .suffix(Self.maximumRecords)
        )
    }

    private func write(_ archive: DiagnosticsArchive) throws {
        try self.fileManager.createDirectory(
            at: self.fileURL.deletingLastPathComponent(),
            withIntermediateDirectories: true
        )
        try Self.encoder.encode(archive).write(to: self.fileURL, options: .atomic)
    }

    private static let encoder: JSONEncoder = {
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .iso8601
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        return encoder
    }()

    private static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }()
}

@MainActor
final class DiagnosticsMetrics: NSObject, MXMetricManagerSubscriber {
    static let shared = DiagnosticsMetrics()
    static let enabledKey = "bobe.diagnostics.enabled"

    private(set) var isSubscribed = false
    private let store: DiagnosticsStore

    override private init() {
        let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        self.store = DiagnosticsStore(
            fileURL: support
                .appending(path: "com.bobe.app", directoryHint: .isDirectory)
                .appending(path: "Diagnostics", directoryHint: .isDirectory)
                .appending(path: "metrics.json", directoryHint: .notDirectory)
        )
        super.init()
    }

    var isEnabled: Bool {
        get { UserDefaults.standard.bool(forKey: Self.enabledKey) }
        set { UserDefaults.standard.set(newValue, forKey: Self.enabledKey) }
    }

    func start() {
        guard self.isEnabled, !self.isSubscribed else { return }
        MXMetricManager.shared.add(self)
        self.isSubscribed = true
    }

    func stop() {
        guard self.isSubscribed else { return }
        MXMetricManager.shared.remove(self)
        self.isSubscribed = false
    }

    func setEnabled(_ enabled: Bool) async throws {
        self.isEnabled = enabled
        if enabled {
            self.start()
        } else {
            self.stop()
            try self.store.delete()
        }
    }

    func export(to destination: URL) throws {
        try self.store.export(to: destination)
    }

    func deleteRetainedData() throws {
        try self.store.delete()
    }

    nonisolated func didReceive(_ payloads: [MXMetricPayload]) {
        let summaries = payloads.map(Self.summary)
        Task { @MainActor in
            do {
                try self.store.append(summaries)
            } catch {
                diagnosticsLogger.error("metric summary storage failed: \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    nonisolated func didReceive(_ payloads: [MXDiagnosticPayload]) {
        let summaries = payloads.map(Self.summary)
        Task { @MainActor in
            do {
                try self.store.append(summaries)
            } catch {
                diagnosticsLogger.error("diagnostic summary storage failed: \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    nonisolated private static func summary(_ payload: MXMetricPayload) -> DiagnosticsSummary {
        let duration = UnitDuration.seconds
        let storage = UnitInformationStorage.bytes
        return DiagnosticsSummary(
            periodStart: payload.timeStampBegin,
            periodEnd: payload.timeStampEnd,
            applicationVersion: payload.latestApplicationVersion,
            includesMultipleApplicationVersions: payload.includesMultipleApplicationVersions,
            cpuSeconds: payload.cpuMetrics?.cumulativeCPUTime.converted(to: duration).value,
            gpuSeconds: payload.gpuMetrics?.cumulativeGPUTime.converted(to: duration).value,
            foregroundSeconds: payload.applicationTimeMetrics?.cumulativeForegroundTime.converted(to: duration).value,
            backgroundSeconds: payload.applicationTimeMetrics?.cumulativeBackgroundTime.converted(to: duration).value,
            peakMemoryBytes: payload.memoryMetrics?.peakMemoryUsage.converted(to: storage).value,
            logicalWriteBytes: payload.diskIOMetrics?.cumulativeLogicalWrites.converted(to: storage).value,
            wifiUploadBytes: payload.networkTransferMetrics?.cumulativeWifiUpload.converted(to: storage).value,
            wifiDownloadBytes: payload.networkTransferMetrics?.cumulativeWifiDownload.converted(to: storage).value,
            cellularUploadBytes: payload.networkTransferMetrics?.cumulativeCellularUpload.converted(to: storage).value,
            cellularDownloadBytes: payload.networkTransferMetrics?.cumulativeCellularDownload.converted(to: storage).value,
            cpuExceptionCount: 0,
            diskWriteExceptionCount: 0,
            hangCount: 0,
            crashCount: 0
        )
    }

    nonisolated private static func summary(_ payload: MXDiagnosticPayload) -> DiagnosticsSummary {
        DiagnosticsSummary(
            periodStart: payload.timeStampBegin,
            periodEnd: payload.timeStampEnd,
            applicationVersion: nil,
            includesMultipleApplicationVersions: false,
            cpuSeconds: nil,
            gpuSeconds: nil,
            foregroundSeconds: nil,
            backgroundSeconds: nil,
            peakMemoryBytes: nil,
            logicalWriteBytes: nil,
            wifiUploadBytes: nil,
            wifiDownloadBytes: nil,
            cellularUploadBytes: nil,
            cellularDownloadBytes: nil,
            cpuExceptionCount: payload.cpuExceptionDiagnostics?.count ?? 0,
            diskWriteExceptionCount: payload.diskWriteExceptionDiagnostics?.count ?? 0,
            hangCount: payload.hangDiagnostics?.count ?? 0,
            crashCount: payload.crashDiagnostics?.count ?? 0
        )
    }
}
