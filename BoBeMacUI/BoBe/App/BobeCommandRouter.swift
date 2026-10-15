import AppKit
import Foundation

enum BobeCommandOutcome: Equatable, Sendable {
    case performed
    case setupRequired
    case unavailable
}

@MainActor
final class BobeCommandRouter {
    static let shared = BobeCommandRouter()

    private let store: BobeStore

    init(store: BobeStore = .shared) {
        self.store = store
    }

    var settingsTitle: String {
        L10n.tr("tray.settings")
    }

    var overlayTitle: String {
        guard SetupWindowManager.shared.isOnboardingCompleted else {
            return L10n.tr("app.setup_incomplete.retry")
        }
        return OverlayWindowManager.shared.isVisible
            ? L10n.tr("tray.hide")
            : L10n.tr("tray.show")
    }

    var captureTitle: String {
        self.store.isCapturing
            ? L10n.tr("tray.capture.disable")
            : L10n.tr("tray.capture.enable")
    }

    var canCheckForUpdates: Bool {
        UpdaterManager.shared.canCheckForUpdates
    }

    @discardableResult
    func openSettings() -> BobeCommandOutcome {
        if SetupWindowManager.shared.isOnboardingCompleted {
            SettingsWindowManager.shared.show()
            return .performed
        }
        SetupWindowManager.shared.show()
        return .setupRequired
    }

    @discardableResult
    func showOverlay() -> BobeCommandOutcome {
        guard SetupWindowManager.shared.isOnboardingCompleted else {
            SetupWindowManager.shared.show()
            return .setupRequired
        }
        guard OverlayWindowManager.shared.panel != nil else { return .unavailable }
        OverlayWindowManager.shared.show()
        return .performed
    }

    @discardableResult
    func hideOverlay() -> BobeCommandOutcome {
        guard SetupWindowManager.shared.isOnboardingCompleted else {
            SetupWindowManager.shared.show()
            return .setupRequired
        }
        guard OverlayWindowManager.shared.panel != nil else { return .unavailable }
        OverlayWindowManager.shared.hide()
        return .performed
    }

    func toggleOverlay() {
        if OverlayWindowManager.shared.isVisible {
            _ = self.hideOverlay()
        } else {
            _ = self.showOverlay()
        }
    }

    func toggleCapture() async {
        guard SetupWindowManager.shared.isOnboardingCompleted else {
            SetupWindowManager.shared.show()
            return
        }
        _ = await self.store.toggleCapture()
    }

    func toggleMicrophone() {
        guard SetupWindowManager.shared.isOnboardingCompleted,
              let url = URL(string: DaemonConfig.baseURL)
        else {
            SetupWindowManager.shared.show()
            return
        }
        Task { await VoicePipeline.shared.toggle(daemonBaseURL: url) }
    }

    @discardableResult
    func toggleLogicalMicrophoneMute() async -> BobeCommandOutcome {
        guard SetupWindowManager.shared.isOnboardingCompleted else { return .setupRequired }
        let desiredMute = !VoicePipeline.shared.muted
        guard await VoicePipeline.shared.setMuted(desiredMute),
              VoicePipeline.shared.muted == desiredMute
        else { return .unavailable }
        return .performed
    }

    @discardableResult
    func stopSpeaking() -> BobeCommandOutcome {
        switch VoicePipeline.shared.state {
        case .thinking, .speaking, .capturing:
            return VoicePipeline.shared.interrupt() ? .performed : .unavailable
        default:
            return .unavailable
        }
    }

    func checkForUpdates() {
        guard self.canCheckForUpdates else { return }
        UpdaterManager.shared.checkForUpdates()
    }

    func showAbout() {
        NSApp.activate()
        NSApp.orderFrontStandardAboutPanel(nil)
    }

    func quit() {
        RunLoop.main.perform(inModes: [.default]) {
            MainActor.assumeIsolated {
                NSApplication.shared.terminate(nil)
            }
        }
    }
}
