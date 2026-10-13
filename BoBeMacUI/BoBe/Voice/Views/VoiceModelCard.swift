import AppKit
import SwiftUI

struct VoiceModelCard: View {
    let name: String
    let purpose: String
    let sizeHint: String
    let location: String?
    let status: Status
    var onInstall: (() async -> Void)?
    var onUninstall: (() async -> Void)?

    @State private var isWorking = false
    @Environment(\.theme) private var theme

    enum Status: Equatable {
        case installed
        case missing
        case downloading(bytesDownloaded: UInt64, bytesTotal: UInt64?, percent: Int?)
        case failed(String)
        case unknown
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                self.statusIcon
                VStack(alignment: .leading, spacing: 2) {
                    Text(self.name)
                        .bobeTextStyle(.rowTitle)
                        .foregroundStyle(self.theme.colors.text)
                    Text(self.purpose)
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer()
                self.statusBadge
            }
            HStack(spacing: 12) {
                Label(self.sizeHint, systemImage: "internaldrive")
                    .font(.system(size: 11))
                    .foregroundStyle(self.theme.colors.textMuted)
                if let location {
                    Button(action: self.openLocationInFinder) {
                        HStack(spacing: 4) {
                            Image(systemName: "folder")
                            Text(self.shortLocation(location))
                                .font(.system(size: 11, design: .monospaced))
                                .lineLimit(1)
                                .truncationMode(.middle)
                        }
                    }
                    .buttonStyle(.plain)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .help(location)
                }
                Spacer()
                self.actionButtons
            }
            if case let .downloading(_, _, percent) = self.status {
                ProgressView(value: Double(percent ?? 0), total: 100)
                    .progressViewStyle(.linear)
                    .tint(self.theme.colors.primary)
            }
            if case let .failed(msg) = self.status {
                Text(msg)
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.error)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: 8)
                .fill(self.theme.colors.surface.opacity(0.4))
        )
        .overlay(
            RoundedRectangle(cornerRadius: 8)
                .stroke(self.theme.colors.border.opacity(0.6), lineWidth: 1)
        )
    }

    private var actionButtons: some View {
        HStack(spacing: 6) {
            switch self.status {
            case .missing, .failed:
                if let onInstall {
                    Button(self.isWorking ? L10n.tr("settings.voice.model.installing") : L10n.tr("settings.voice.model.install")) {
                        self.runAction(onInstall)
                    }
                    .bobeButton(.primary, size: .small)
                    .disabled(self.isWorking)
                }
            case .installed:
                if let onUninstall {
                    Button(self.isWorking ? L10n.tr("settings.voice.model.uninstalling") : L10n.tr("settings.voice.model.uninstall")) {
                        self.runAction(onUninstall)
                    }
                    .bobeButton(.ghost, size: .small)
                    .disabled(self.isWorking)
                }
            case .downloading, .unknown:
                EmptyView()
            }
        }
    }

    private func runAction(_ action: @escaping () async -> Void) {
        self.isWorking = true
        Task {
            await action()
            self.isWorking = false
        }
    }

    private var statusIcon: some View {
        Group {
            switch self.status {
            case .installed:
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(self.theme.colors.success)
            case .missing:
                Image(systemName: "exclamationmark.circle.fill")
                    .foregroundStyle(self.theme.colors.warning)
            case .downloading:
                BobeSpinner(size: 14)
            case .failed:
                Image(systemName: "xmark.octagon.fill")
                    .foregroundStyle(self.theme.colors.error)
            case .unknown:
                Image(systemName: "questionmark.circle")
                    .foregroundStyle(self.theme.colors.textMuted)
            }
        }
        .font(.system(size: 18))
        .frame(width: 22, height: 22)
        .accessibilityHidden(true)
    }

    private var statusBadge: some View {
        Text(self.statusLabel)
            .font(.system(size: 11, weight: .medium))
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(
                Capsule().fill(self.statusBadgeColor.opacity(0.15))
            )
            .foregroundStyle(self.statusBadgeColor)
    }

    private var statusLabel: String {
        switch self.status {
        case .installed: return L10n.tr("settings.voice.model.status.installed")
        case .missing: return L10n.tr("settings.voice.model.status.missing")
        case let .downloading(down, total, percent):
            if let percent {
                return L10n.tr("settings.voice.model.status.downloading_percent_format", percent)
            }
            if let total {
                return L10n.tr(
                    "settings.voice.model.status.downloading_bytes_format",
                    self.format(down),
                    self.format(total)
                )
            }
            return L10n.tr("settings.voice.model.status.downloading")
        case .failed: return L10n.tr("settings.voice.model.status.failed")
        case .unknown: return L10n.tr("settings.voice.model.status.checking")
        }
    }

    private var statusBadgeColor: Color {
        switch self.status {
        case .installed: self.theme.colors.success
        case .missing: self.theme.colors.warning
        case .failed: self.theme.colors.error
        case .downloading: self.theme.colors.primary
        case .unknown: self.theme.colors.textMuted
        }
    }

    private func shortLocation(_ location: String) -> String {
        location
            .replacingOccurrences(of: NSHomeDirectory(), with: "~")
    }

    private func format(_ bytes: UInt64) -> String {
        let mb = Double(bytes) / 1_048_576.0
        return String(format: "%.0f MB", mb)
    }

    private func openLocationInFinder() {
        guard let location = self.location else { return }
        let expanded = NSString(string: location).expandingTildeInPath
        let url = URL(fileURLWithPath: expanded)
        if FileManager.default.fileExists(atPath: url.path) {
            NSWorkspace.shared.activateFileViewerSelecting([url])
        } else {
            NSWorkspace.shared.open(url.deletingLastPathComponent())
        }
    }
}
