import AppKit
import Foundation
import OSLog
import Sparkle

nonisolated private let updaterLogger = Logger(subsystem: "com.bobe.app", category: "Updater")

final class UpdaterManager: NSObject, SPUUpdaterDelegate {
    static let shared = UpdaterManager()

    private lazy var controller = SPUStandardUpdaterController(
        startingUpdater: false,
        updaterDelegate: self,
        userDriverDelegate: nil
    )

    override private init() {
        super.init()
    }

    var isConfigured: Bool {
        self.feedURLString != nil
    }

    var canCheckForUpdates: Bool {
        self.isConfigured && self.controller.updater.canCheckForUpdates
    }

    func setup() {
        _ = self.controller

        guard self.isConfigured else {
            updaterLogger.info("Sparkle disabled: no valid SUFeedURL configured")
            return
        }

        do {
            try self.controller.updater.start()
        } catch {
            updaterLogger.warning("Sparkle failed to start: \(error.localizedDescription)")
        }
    }

    func checkForUpdates() {
        guard self.isConfigured else {
            updaterLogger.warning("Updates not configured: missing or placeholder SUFeedURL")
            return
        }

        if !self.controller.updater.sessionInProgress {
            do {
                try self.controller.updater.start()
            } catch {
                updaterLogger.warning("Sparkle failed to start: \(error.localizedDescription)")
                return
            }
        }

        guard self.controller.updater.canCheckForUpdates else {
            return
        }
        self.controller.checkForUpdates(nil)
    }

    func feedURLString(for updater: SPUUpdater) -> String? {
        self.feedURLString
    }

    func updater(_ updater: SPUUpdater, didAbortWithError error: Error) {
        let nsError = error as NSError
        if nsError.domain == NSURLErrorDomain {
            updaterLogger.info("Update check skipped (network): \(error.localizedDescription)")
            return
        }
        updaterLogger.warning("Sparkle error: \(error.localizedDescription)")
    }

    private var feedURLString: String? {
        guard let rawFeedURL = Bundle.main.object(forInfoDictionaryKey: "SUFeedURL") as? String else {
            return nil
        }
        let trimmed = rawFeedURL.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed != "https://example.com/appcast.xml" else {
            return nil
        }
        return trimmed
    }
}
