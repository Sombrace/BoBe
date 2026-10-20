import AVFoundation
import SwiftUI

struct MicButton: View {
    @Environment(\.theme) private var theme
    @State private var pipeline = VoicePipeline.shared
    @State private var pollTask: Task<Void, Never>?

    var body: some View {
        Button(action: self.handleTap) {
            SatelliteChrome(systemName: self.icon, isActive: self.isLive, isDimmed: self.unusableDim)
                .overlay {
                    if self.isSttBusy {
                        let displayed = max(0.03, self.installFraction)
                        Circle()
                            .trim(from: 0, to: displayed)
                            .stroke(
                                self.theme.colors.primary,
                                style: StrokeStyle(lineWidth: 2.5, lineCap: .round)
                            )
                            .rotationEffect(.degrees(-90))
                            .frame(width: AvatarMetrics.satelliteSize + 5, height: AvatarMetrics.satelliteSize + 5)
                            .animation(BobeMotion.soft, value: displayed)
                            .accessibilityElement()
                            .accessibilityLabel(L10n.tr("overlay.input.mic.install_progress.accessibility"))
                            .accessibilityValue(
                                String(
                                    format: L10n.tr("overlay.input.mic.install_progress.value_format"),
                                    Int((self.installFraction * 100).rounded())
                                )
                            )
                    }
                }
                .overlay(alignment: .topTrailing) {
                    if self.showsProblemBadge, !self.isSttBusy {
                        Image(systemName: self.badgeIcon)
                            .font(.system(size: 11))
                            .foregroundStyle(self.theme.colors.primary)
                            .background(Circle().fill(self.theme.colors.surface).frame(width: 12, height: 12))
                            .offset(x: 2, y: -2)
                    }
                }
        }
        .buttonStyle(BobePressStyle(scale: 0.9))
        .accessibilityLabel(self.accessibilityLabel)
        .accessibilityIdentifier("overlay.avatar.mic-toggle")
        .help(self.tooltip)
        .disabled(self.isDisabled)
        .task {
            self.pipeline.updatePermission(
                AVCaptureDevice.authorizationStatus(for: .audio)
            )
            await self.pipeline.refreshDaemonState()
            await self.pipeline.bootstrapSttPresence()
            if self.pipeline.readiness == .ready {
                await self.pipeline.prewarmSttOnly()
            }
        }
        .onDisappear { self.pollTask?.cancel() }
    }

    private var isLive: Bool {
        guard !self.needsSetup else { return false }
        switch self.pipeline.state {
        case .listening, .capturing: return true
        default: return false
        }
    }

    private var needsSetup: Bool {
        switch self.pipeline.readiness {
        case .disabledByUser, .modelsMissing, .failed: true
        default: false
        }
    }

    private var accessibilityLabel: String {
        switch self.pipeline.readiness {
        case .permissionMissing:
            L10n.tr("overlay.input.mic.permission_denied")
        case .modelsMissing:
            L10n.tr("overlay.input.mic.tap_to_install")
        case .disabledByUser, .failed:
            L10n.tr("overlay.input.mic.needs_setup")
        default:
            L10n.tr("overlay.input.mic.accessibility")
        }
    }

    private var tooltip: String {
        switch self.pipeline.readiness {
        case .permissionMissing:
            return L10n.tr("overlay.input.mic.permission_denied")
        case .modelsMissing:
            return L10n.tr("overlay.input.mic.tap_to_install")
        case .disabledByUser:
            return L10n.tr("overlay.input.mic.needs_setup")
        case .installing:
            let percent = Int((self.installFraction * 100).rounded())
            return L10n.tr("overlay.input.mic.installing", percent)
        case let .failed(msg):
            return L10n.tr("overlay.input.mic.failed_format", msg)
        case .preparing, .ready:
            break
        }
        if let err = self.pipeline.lastError, !err.isEmpty {
            return err
        }
        switch self.pipeline.state {
        case .connecting: return L10n.tr("overlay.input.mic.disabled.connecting")
        case .thinking: return L10n.tr("overlay.input.mic.disabled.thinking")
        case .speaking: return L10n.tr("overlay.input.mic.disabled.speaking")
        case .cancelling: return L10n.tr("overlay.input.mic.disabled.cancelling")
        default: return L10n.tr("overlay.input.mic.accessibility")
        }
    }

    private func handleTap() {
        if self.pipeline.terminalServerErrorCode == "remote_audio_consent_required",
           !RemoteSpeechConsent.shared.currentAuthorityEnabled {
            self.openVoiceSettings()
            return
        }
        switch self.pipeline.readiness {
        case .modelsMissing, .installing, .disabledByUser, .failed:
            self.openVoiceSettings()
            return
        case .permissionMissing, .preparing, .ready:
            break
        }
        switch self.pipeline.permission {
        case .authorized:
            Task {
                guard let url = URL(string: DaemonConfig.baseURL) else { return }
                await self.pipeline.toggle(daemonBaseURL: url)
            }
        case .notDetermined:
            Task {
                let granted = await AVCaptureDevice.requestAccess(for: .audio)
                self.pipeline.updatePermission(granted ? .authorized : .denied)
                if granted, self.pipeline.readiness == .ready {
                    await self.pipeline.prewarmSttOnly()
                }
            }
        case .denied, .restricted:
            if let url = URL(
                string: "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_Microphone"
            ) {
                NSWorkspace.shared.open(url)
            }
        @unknown default:
            break
        }
    }

    private func openVoiceSettings() {
        SettingsWindowManager.shared.show(initialCategory: .voice)
        self.schedulePoll()
    }

    private func schedulePoll() {
        self.pollTask?.cancel()
        self.pollTask = Task {
            for _ in 0 ..< 60 where !Task.isCancelled {
                try? await Task.sleep(for: .seconds(30))
                await self.pipeline.refreshDaemonState()
                switch VoiceModelInstaller.shared.preparation {
                case .ready, .failed: return
                case .preparing, .installing, .missing: break
                }
            }
        }
    }

    private var icon: String {
        if self.pipeline.readiness == .permissionMissing {
            return "mic.slash.fill"
        }
        if self.needsSetup {
            return "mic.badge.plus"
        }
        return switch self.pipeline.state {
        case .idle, .failed: "mic.fill"
        case .listening, .capturing: "waveform"
        case .connecting, .thinking, .speaking, .cancelling: "mic.fill"
        }
    }

    private var isSttBusy: Bool {
        self.pipeline.readiness == .installing
    }

    private var unusableDim: Bool {
        switch self.pipeline.readiness {
        case .ready, .preparing: false
        case .disabledByUser, .permissionMissing, .installing, .modelsMissing, .failed: true
        }
    }

    private var isDisabled: Bool {
        switch self.pipeline.readiness {
        case .ready: break
        case .disabledByUser, .modelsMissing, .failed, .permissionMissing, .installing: return false
        case .preparing: return true
        }
        return switch self.pipeline.state {
        case .connecting, .thinking, .speaking, .cancelling: true
        default: false
        }
    }

    private var installFraction: CGFloat {
        let ttsPercent: Double = {
            guard let models = self.pipeline.installSnapshot?.models else { return 0 }
            let percents = models.compactMap(\.percent)
            guard !percents.isEmpty else { return 0 }
            return Double(percents.reduce(0, +)) / Double(percents.count) / 100.0
        }()
        let sttFraction: Double = (self.pipeline.sttStatus == .ready) ? 1.0 : 0.0
        let combined = min(ttsPercent, sttFraction)
        return CGFloat(max(combined, ttsPercent > 0 ? ttsPercent * 0.5 : 0.04))
    }

    private var badgeIcon: String {
        "exclamationmark.circle.fill"
    }

    private var showsProblemBadge: Bool {
        if case .failed = self.pipeline.readiness { return true }
        return false
    }
}
