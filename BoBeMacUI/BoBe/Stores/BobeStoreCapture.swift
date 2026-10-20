import AppKit
import CoreGraphics
import Foundation

extension BobeStore {
    func synchronizeCaptureStartup() {
        let selection = self.selection
        let settingsStore = self.settingsStore
        self.captureStartupTask?.cancel()
        self.captureStartupTask = Task { [weak self] in
            await settingsStore.refresh()
            guard let self, self.isCurrent(selection), !Task.isCancelled,
                  settingsStore.settings?.captureEnabled == true,
                  !CGPreflightScreenCaptureAccess()
            else { return }
            CGRequestScreenCaptureAccess()
            bobeStoreLogger.warning("Screen capture permission not granted")
        }
    }

    func handleBodyRequest(_ payload: BodyRequestPayload) {
        let client = DaemonClient(endpoint: self.selection.endpoint)
        let requestId = payload.requestId
        guard payload.action == BodyRequestWire.screenRead else {
            Task { try? await client.completeBodyRequest(requestId, reply: .failed("unsupported_action")) }
            return
        }
        guard self.screenReadTask == nil else {
            Task { try? await client.completeBodyRequest(requestId, reply: .declined("busy")) }
            return
        }
        let looks = CGPreflightScreenCaptureAccess()
        if looks {
            self.isReadingScreen = true
        }
        self.screenReadTask = Task { [weak self] in
            let started = ContinuousClock.now
            let reply = await ScreenReader.read()
            do {
                try await client.completeBodyRequest(requestId, reply: reply)
            } catch {
                bobeStoreLogger.warning("Screen reading reply was not delivered: \(error.localizedDescription)")
            }
            if reply == .declined(ScreenReader.permissionMissing) {
                self?.noteScreenUnavailable()
            }
            if looks {
                let glance = Duration.milliseconds(StoreTiming.screenGlanceMilliseconds)
                let elapsed = ContinuousClock.now - started
                if elapsed < glance {
                    try? await Task.sleep(for: glance - elapsed)
                }
            }
            self?.isReadingScreen = false
            self?.screenReadTask = nil
        }
    }

    func noteScreenUnavailable() {
        guard !self.screenUnavailableNoticeShown else { return }
        self.screenUnavailableNoticeShown = true
        self.screenUnavailableNotice = true
    }

    func dismissScreenUnavailableNotice() {
        self.screenUnavailableNotice = false
    }

    func openScreenRecordingSettings() {
        self.screenUnavailableNotice = false
        if let url = URL(string: "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_ScreenCapture") {
            NSWorkspace.shared.open(url)
        }
    }

    func handleConnectionChange(_ state: RuntimeConnectionState) async {
        switch state {
        case .connected:
            self.repairIssue = nil
            self.updateState { ctx in
                ctx.daemonConnected = true
                ctx.daemonError = false
            }
            self.cancelReconnectStatusTransition()
            self.isReconnecting = false
            self.hasConnectedOnce = true
            self.synchronizeCaptureStartup()
            await self.synchronizeStatusAndConversation()

        case .disconnected:
            self.updateState { $0.daemonConnected = false }
            guard self.hasConnectedOnce else { return }
            self.scheduleReconnectStatusTransition()

        case let .repairRequired(issue):
            self.cancelReconnectStatusTransition()
            self.isReconnecting = false
            self.repairIssue = issue
            self.updateState { ctx in
                ctx.daemonConnected = false
                ctx.daemonError = true
            }
            _ = SetupWindowManager.shared.showRuntimeRepair(issue: issue)
        }
    }

    private func synchronizeStatusAndConversation() async {
        let selection = self.selection
        let client = DaemonClient(endpoint: selection.endpoint)
        var replacingActiveStream = false
        do {
            let status = try await client.getStatus()
            guard self.isCurrent(selection) else { return }
            replacingActiveStream = status.indicatorType == .idle
            self.updateState { ctx in
                ctx.acceptingUserMessages = status.acceptingUserMessages
                let indicator = status.indicatorType
                ctx.thinking = indicator == .thinking
                ctx.speaking = indicator == .streaming
            }
        } catch {
            bobeStoreLogger.warning("Status sync skipped: \(error.localizedDescription)")
        }
        guard self.isCurrent(selection) else { return }
        await self.reloadCanonicalConversation(
            replacingActiveStream: replacingActiveStream
        )
    }

    func scheduleReconnectStatusTransition() {
        self.reconnectStatusTask?.cancel()
        self.reconnectStatusTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(StoreTiming.reconnectStatusDelayMilliseconds))
            guard let self, !Task.isCancelled, !self.context.daemonConnected else { return }
            self.isReconnecting = true
        }
    }

    func cancelReconnectStatusTransition() {
        self.reconnectStatusTask?.cancel()
        self.reconnectStatusTask = nil
    }
}
