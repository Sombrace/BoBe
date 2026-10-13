import AppKit
import AVFoundation
import CoreGraphics
import SwiftUI

private enum PermissionState {
    case unknown
    case granted
    case denied
}

private struct PermissionItem {
    let symbol: String
    let title: String
    let subtitle: String
    let deniedHint: String
    let identifier: String
    let settingsPane: String
}

struct PermissionsStepView: View {
    let screenContextEnabled: Bool
    let onContinue: () -> Void

    init(screenContextEnabled: Bool = true, onContinue: @escaping () -> Void) {
        self.screenContextEnabled = screenContextEnabled
        self.onContinue = onContinue
    }

    @State private var screenState: PermissionState = .unknown
    @State private var micState: PermissionState = .unknown
    @AppStorage("screenCapturePermissionRequested") private var screenPermissionRequested = false
    @Environment(\.theme) private var theme

    private var allGranted: Bool {
        (!self.screenContextEnabled || self.screenState == .granted) && self.micState == .granted
    }

    private var screenSubtitle: String {
        let endpoint = DaemonConfig.endpoint
        if !endpoint.isLoopbackRuntime, let host = endpoint.baseURL.host {
            return L10n.tr("setup.permissions.card.screen.subtitle_remote_format", host)
        }
        return L10n.tr("setup.permissions.card.screen.subtitle")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            SetupStepHeader(
                title: L10n.tr(
                    self.screenContextEnabled
                        ? "setup.permissions.title"
                        : "setup.permissions.title.no_capture"
                ),
                message: L10n.tr(
                    self.screenContextEnabled
                        ? "setup.permissions.body"
                        : "setup.permissions.body.no_capture"
                )
            )
            .padding(.top, 8)

            VStack(spacing: 10) {
                if self.screenContextEnabled {
                    self.permissionCard(
                        PermissionItem(
                            symbol: "viewfinder",
                            title: L10n.tr("setup.permissions.card.screen.title"),
                            subtitle: self.screenSubtitle,
                            deniedHint: L10n.tr("setup.permissions.card.screen.denied"),
                            identifier: "screen",
                            settingsPane: "Privacy_ScreenCapture"
                        ),
                        state: self.screenState,
                        request: self.requestScreen
                    )
                    .setupStagger(0)
                }
                self.permissionCard(
                    PermissionItem(
                        symbol: "mic.fill",
                        title: L10n.tr("setup.permissions.card.mic.title"),
                        subtitle: L10n.tr("setup.permissions.card.mic.subtitle"),
                        deniedHint: L10n.tr("setup.permissions.card.mic.denied"),
                        identifier: "microphone",
                        settingsPane: "Privacy_Microphone"
                    ),
                    state: self.micState,
                    request: self.requestMicrophone
                )
                .setupStagger(1)
            }

            Spacer(minLength: 0)

            Button(
                L10n.tr(self.allGranted ? "setup.welcome.continue" : "setup.permissions.skip"),
                action: self.onContinue
            )
            .bobeButton(self.allGranted ? .primary : .ghost, size: .regular)
            .keyboardShortcut(self.allGranted ? .defaultAction : .cancelAction)
            .accessibilityIdentifier("setup.permissions.continue")
            .frame(maxWidth: .infinity, alignment: .trailing)
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            self.refreshState()
        }
        .task { self.refreshState() }
        .animation(BobeMotion.standard, value: self.screenState)
        .animation(BobeMotion.standard, value: self.micState)
    }

    private func permissionCard(
        _ item: PermissionItem,
        state: PermissionState,
        request: @escaping () -> Void
    ) -> some View {
        HStack(alignment: .center, spacing: 13) {
            BobeIconTile(systemName: item.symbol, tint: self.iconColor(for: state), size: 36)
            VStack(alignment: .leading, spacing: 2) {
                Text(item.title)
                    .bobeTextStyle(.heading)
                    .foregroundStyle(self.theme.colors.text)
                Text(state == .denied ? item.deniedHint : item.subtitle)
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 8)
            switch state {
            case .granted:
                Label(L10n.tr("setup.permissions.card.status.allowed"), systemImage: "checkmark.circle.fill")
                    .labelStyle(.iconOnly)
                    .font(.system(size: 20))
                    .foregroundStyle(self.theme.colors.success)
                    .help(L10n.tr("setup.permissions.card.status.allowed"))
                    .transition(.scale.combined(with: .opacity))
            case .unknown:
                Button(L10n.tr("setup.permissions.grant"), action: request)
                    .bobeButton(.primary, size: .small)
                    .accessibilityIdentifier("setup.permissions.\(item.identifier).allow")
            case .denied:
                Button(L10n.tr("setup.permissions.open_settings")) {
                    self.openSettings(pane: item.settingsPane)
                }
                .bobeButton(.secondary, size: .small)
                .accessibilityIdentifier("setup.permissions.\(item.identifier).settings")
            }
        }
        .padding(14)
        .bobeRaised(cornerRadius: BobeRadius.card)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(item.title)
    }

    private func iconColor(for state: PermissionState) -> Color {
        switch state {
        case .unknown: self.theme.colors.textMuted
        case .granted: self.theme.colors.success
        case .denied: self.theme.colors.warning
        }
    }

    private func refreshState() {
        if !self.screenContextEnabled || CGPreflightScreenCaptureAccess() {
            self.screenState = .granted
        } else {
            self.screenState = self.screenPermissionRequested ? .denied : .unknown
        }
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized: self.micState = .granted
        case .denied, .restricted: self.micState = .denied
        case .notDetermined: self.micState = .unknown
        @unknown default: self.micState = .unknown
        }
    }

    private func requestScreen() {
        self.screenPermissionRequested = true
        self.screenState = CGRequestScreenCaptureAccess() ? .granted : .denied
    }

    private func requestMicrophone() {
        Task {
            let granted = await AVCaptureDevice.requestAccess(for: .audio)
            await MainActor.run {
                self.micState = granted ? .granted : .denied
            }
        }
    }

    private func openSettings(pane: String) {
        if let url = URL(string: "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?\(pane)") {
            NSWorkspace.shared.open(url)
        }
    }
}
