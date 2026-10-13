import AppKit
import OSLog
import SwiftUI

private let logger = Logger(subsystem: "com.bobe.app", category: "App")

@main
struct BoBeApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) var appDelegate
    @State private var store = BobeStore.shared

    var body: some Scene {
        Settings {
            EmptyView()
        }
        .commands {
            CommandGroup(replacing: .appSettings) {
                Button(BobeCommandRouter.shared.settingsTitle) {
                    BobeCommandRouter.shared.openSettings()
                }
                .keyboardShortcut(",", modifiers: .command)
            }
            CommandGroup(after: .toolbar) {
                Button(BobeCommandRouter.shared.overlayTitle) {
                    BobeCommandRouter.shared.toggleOverlay()
                }
                .keyboardShortcut("b", modifiers: [.command, .shift])
            }
            CommandMenu(L10n.tr("menu.voice.title")) {
                Button(L10n.tr("menu.voice.toggle_mic")) {
                    BobeCommandRouter.shared.toggleMicrophone()
                }
                .keyboardShortcut("m", modifiers: [.command, .shift])

                Button(L10n.tr("menu.voice.stop_speaking")) {
                    BobeCommandRouter.shared.stopSpeaking()
                }
                .keyboardShortcut(".", modifiers: .command)
            }
        }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    private let store: BobeStore
    private let statusItemController: BobeStatusItemController
    private var isQuitting = false
    private static let shutdownDeadlineSeconds: TimeInterval = 8
    private var isDuplicateInstance = false
    private var isStartingUp = true
    private let isBodyAdapterOnly =
        ProcessInfo.processInfo.environment["BOBE_BODY_ADAPTER_ONLY"] == "1"

    override init() {
        self.store = BobeStore.shared
        self.statusItemController = BobeStatusItemController(store: self.store)
        super.init()
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        if NSClassFromString("XCTestCase") != nil {
            return
        }
        logger.info("BoBe starting up")

        if self.isBodyAdapterOnly {
            NSApp.setActivationPolicy(.prohibited)
            self.isStartingUp = false
            guard let configuration = BodyAdapterConfig.load() else {
                logger.error("Body adapter-only mode has invalid configuration")
                NSApp.terminate(nil)
                return
            }
            Task {
                await BodySpeechAdapter.shared.start(configuration: configuration)
            }
            return
        }

        self.store.applyPersistedLocale()

        NSApp.setActivationPolicy(.accessory)

        let currentPID = ProcessInfo.processInfo.processIdentifier
        let bundleMatches = NSRunningApplication.runningApplications(
            withBundleIdentifier: "com.bobe.app"
        )
        .filter { $0.processIdentifier != currentPID }

        var candidates = bundleMatches
        for app in NSWorkspace.shared.runningApplications
            where app.processIdentifier != currentPID
            && (app.executableURL?.lastPathComponent == "BoBe" || app.localizedName == "BoBe")
            && !candidates.contains(where: { $0.processIdentifier == app.processIdentifier }) {
            candidates.append(app)
        }

        if let existing = candidates.first {
            logger.warning("Another BoBe instance detected (pid: \(existing.processIdentifier)) — activating it and exiting")
            self.isDuplicateInstance = true
            existing.activate()
            NSApp.terminate(nil)
            return
        }

        self.statusItemController.setup()
        UpdaterManager.shared.setup()
        SystemPowerObserver.shared.start()
        DiagnosticsMetrics.shared.start()

        NotificationCenter.default.addObserver(
            self,
            selector: #selector(self.handleWelcomeCompleted),
            name: .bobeWelcomeCompleted,
            object: nil
        )

        Task { @MainActor in
            await self.startApp()
        }
    }

    @objc
    private func handleWelcomeCompleted() {
        Task { @MainActor in
            SetupWindowManager.shared.close()
            await self.completePostStartSetup()
            self.showOverlay()
            self.store.connect()
        }
    }

    private func completePostStartSetup() async {
        if let warning = await BackendService.shared.startupWarning {
            self.store.surfaceWarning(warning)
        }
        if let bodyAdapter = BodyAdapterConfig.load() {
            await BodySpeechAdapter.shared.start(configuration: bodyAdapter)
        }
        await self.repromptVoiceSetupIfModelsMissing()
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if self.isDuplicateInstance { return .terminateNow }
        guard !self.isQuitting else { return .terminateLater }
        self.isQuitting = true
        if self.isBodyAdapterOnly {
            Task {
                await BodySpeechAdapter.shared.stop()
                await MainActor.run {
                    NSApp.reply(toApplicationShouldTerminate: true)
                }
            }
            return .terminateLater
        }

        OverlayWindowManager.shared.hide()
        self.statusItemController.tearDown()
        DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + Self.shutdownDeadlineSeconds) {
            logger.warning("Hard shutdown timeout — forcing exit")
            exit(0)
        }

        Task {
            self.store.beginShutdown()
            await SettingsStore.shared.flushPendingSave()

            await MainActor.run {
                SystemPowerObserver.shared.stop()
                DiagnosticsMetrics.shared.stop()
            }

            await BodySpeechAdapter.shared.stop()
            self.store.disconnect()
            await BackendService.shared.stop()

            await MainActor.run {
                OverlayWindowManager.shared.close()
                SettingsWindowManager.shared.close()
                NSApp.reply(toApplicationShouldTerminate: true)
            }
        }
        return .terminateLater
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag {
            if SetupWindowManager.shared.isOnboardingCompleted {
                OverlayWindowManager.shared.show()
            } else {
                SetupWindowManager.shared.show()
            }
        }
        return true
    }

    func applicationDidBecomeActive(_ notification: Notification) {
        guard !self.isBodyAdapterOnly else { return }
        guard !self.isQuitting, !self.isStartingUp else { return }
        if SetupWindowManager.shared.isOnboardingCompleted {
            if OverlayWindowManager.shared.panel == nil {
                Task { @MainActor in self.showOverlay() }
            }
        } else {
            OverlayWindowManager.shared.close()
            SetupWindowManager.shared.show()
        }
    }

    @MainActor
    private func startApp() async {
        defer { self.isStartingUp = false }
        let isDev = ProcessInfo.processInfo.environment["BOBE_DEV"] != nil

        guard EndpointStore.shared.endpoint != nil else {
            SetupWindowManager.shared.show()
            return
        }

        if DaemonConfig.endpoint.managesLocalProcess, !isDev {
            var serviceStarted = false
            var attempt = 0
            var retryRounds = 0
            while !serviceStarted {
                attempt += 1
                do {
                    try await BackendService.shared.start()
                    serviceStarted = true
                    retryRounds = 0
                } catch {
                    logger.error("Backend start attempt \(attempt) failed: \(error.localizedDescription, privacy: .public)")
                    if attempt >= 3 {
                        retryRounds += 1
                        let shouldRetry = await showServiceErrorDialog(
                            error: error,
                            retryRounds: retryRounds
                        )
                        guard shouldRetry else {
                            BobeCommandRouter.shared.quit()
                            return
                        }
                        attempt = 0
                        continue
                    }
                }
            }
        } else if !DaemonConfig.endpoint.managesLocalProcess {
            logger.info("Remote daemon mode: skipping local process management")
            do {
                try await BackendService.shared.start()
            } catch {
                logger.error("Remote backend unavailable: \(error.localizedDescription, privacy: .public)")
                self.showRuntimeConnectionRepair(error)
                return
            }
        } else {
            logger.info("Dev mode: skipping service management (run `bobe serve` manually)")
        }

        while EndpointStore.shared.endpoint != nil {
            do {
                _ = try await EndpointAttachService.shared.attachConfiguredEndpoint()
                break
            } catch {
                logger.error("Runtime attach verification failed: \(error.localizedDescription, privacy: .public)")
                self.showRuntimeConnectionRepair(error)
                return
            }
        }

        if let warning = await BackendService.shared.startupWarning {
            self.store.surfaceWarning(warning)
        }
        if let bodyAdapter = BodyAdapterConfig.load() {
            await BodySpeechAdapter.shared.start(configuration: bodyAdapter)
        } else if ProcessInfo.processInfo.environment["BOBE_BODY_ADAPTER"] == "1" {
            logger.error(
                "BOBE_BODY_ADAPTER requires a loopback URL and BOBE_BODY__ADAPTER_TOKEN"
            )
        }
        if SetupWindowManager.shared.isOnboardingCompleted {
            self.showOverlay()
            self.store.connect()
            Task { @MainActor in
                await self.repromptVoiceSetupIfModelsMissing()
            }
        } else {
            SetupWindowManager.shared.show()
        }
    }

    private func repromptVoiceSetupIfModelsMissing() async {
        guard !VoiceSetupPreference.isDeferred else { return }
        let pipeline = VoicePipeline.shared
        await pipeline.refreshDaemonState()
        pipeline.bootstrapSttPresence()
        switch pipeline.readiness {
        case .modelsMissing, .failed:
            let readinessDesc = String(describing: pipeline.readiness)
            logger.warning(
                "voice models missing post-onboarding — re-opening voice setup (readiness=\(readinessDesc, privacy: .public))"
            )
            SetupWindowManager.shared.show(initialStep: .voiceSetup)
        case .ready, .preparing, .installing, .disabledByUser, .permissionMissing:
            return
        }
    }

    @MainActor
    private func showOverlay() {
        let theme = ThemeStore.shared.currentTheme
        let model = OverlayModel()
        OverlayWindowManager.shared.createPanels(
            avatar: OverlayView(surface: .avatar, model: model, store: self.store).environment(\.theme, theme),
            column: OverlayView(surface: .column, model: model, store: self.store).environment(\.theme, theme)
        )
    }

    private func showRuntimeConnectionRepair(_ error: any Error) {
        let issue = RuntimeIssue(error)
        guard SetupWindowManager.shared.isOnboardingCompleted else {
            SetupWindowManager.shared.show(initialStep: .runtimeLocation, runtimeRepairIssue: issue)
            return
        }
        SetupWindowManager.shared.show(
            initialStep: .runtimeLocation,
            runtimeRepairIssue: issue,
            completesAfterRuntimeRepair: true,
            marksOnboardingComplete: false,
            onComplete: { [weak self] in
                SetupWindowManager.shared.close()
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    await self.startApp()
                }
            }
        )
    }

    @MainActor
    private func showServiceErrorDialog(error: Error, retryRounds: Int) async -> Bool {
        let isPersistent = retryRounds >= 2
        let summary = isPersistent
            ? L10n.tr("app.service_error.persistent_summary")
            : L10n.tr("app.service_error.summary")

        var details = L10n.tr("app.service_error.detail_prefix")
            + error.localizedDescription
        if let stderr = await BackendService.shared.lastError {
            details += "\n\n" + L10n.tr("app.service_error.backend_output_prefix") + stderr
        }
        details += "\n\n" + L10n.tr("app.service_error.logs_hint")

        let alert = NSAlert()
        alert.messageText = L10n.tr("app.service_error.title")
        alert.informativeText = summary + "\n\n" + details
        alert.alertStyle = .critical
        alert.addButton(withTitle: L10n.tr("app.common.retry"))
        alert.addButton(withTitle: L10n.tr("app.common.quit"))
        return alert.runModal() == .alertFirstButtonReturn
    }
}
