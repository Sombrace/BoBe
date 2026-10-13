@testable import BoBe
import Foundation
import Testing

@Suite("Local diagnostics")
struct DiagnosticsMetricsTests {
    @Test
    func retentionDropsOldFutureAndExcessRecords() {
        let now = Date(timeIntervalSince1970: 2_000_000_000)
        let recent = (0 ..< 35).map { index in
            Self.summary(ending: now.addingTimeInterval(TimeInterval(index - 34) * 60))
        }
        let old = Self.summary(ending: now.addingTimeInterval(-DiagnosticsStore.maximumAge - 1))
        let future = Self.summary(ending: now.addingTimeInterval(1))

        let retained = DiagnosticsStore.retained([old] + recent + [future], now: now)

        #expect(retained.count == DiagnosticsStore.maximumRecords)
        #expect(retained.first?.periodEnd == recent[5].periodEnd)
        #expect(retained.last?.periodEnd == recent[34].periodEnd)
    }

    @Test
    func archiveRoundTripsExportsAndDeletesUsingRealFiles() throws {
        let root = FileManager.default.temporaryDirectory
            .appending(path: "bobe-diagnostics-tests-\(UUID().uuidString)", directoryHint: .isDirectory)
        let retainedURL = root.appending(path: "retained/metrics.json")
        let exportURL = root.appending(path: "export.json")
        defer { try? FileManager.default.removeItem(at: root) }

        let now = Date(timeIntervalSince1970: 2_000_000_000)
        let summary = Self.summary(ending: now)
        let store = DiagnosticsStore(fileURL: retainedURL)
        try store.append([summary], now: now)
        #expect(try store.load(now: now).summaries == [summary])

        try store.export(to: exportURL, now: now)
        let exported = try String(contentsOf: exportURL, encoding: .utf8)
        #expect(exported.contains("cpuSeconds"))
        #expect(!exported.localizedCaseInsensitiveContains("transcript"))
        #expect(!exported.localizedCaseInsensitiveContains("prompt"))
        #expect(!exported.localizedCaseInsensitiveContains("callStack"))
        #expect(!exported.localizedCaseInsensitiveContains("credential"))
        #expect(!exported.localizedCaseInsensitiveContains("token"))
        #expect(!exported.contains(FileManager.default.homeDirectoryForCurrentUser.path))

        try store.delete()
        #expect(!FileManager.default.fileExists(atPath: retainedURL.path))
    }

    private static func summary(ending periodEnd: Date) -> DiagnosticsSummary {
        DiagnosticsSummary(
            periodStart: periodEnd.addingTimeInterval(-60),
            periodEnd: periodEnd,
            applicationVersion: "1.0",
            includesMultipleApplicationVersions: false,
            cpuSeconds: 1,
            gpuSeconds: 2,
            foregroundSeconds: 3,
            backgroundSeconds: 4,
            peakMemoryBytes: 5,
            logicalWriteBytes: 6,
            wifiUploadBytes: 7,
            wifiDownloadBytes: 8,
            cellularUploadBytes: 9,
            cellularDownloadBytes: 10,
            cpuExceptionCount: 0,
            diskWriteExceptionCount: 0,
            hangCount: 0,
            crashCount: 0
        )
    }
}
