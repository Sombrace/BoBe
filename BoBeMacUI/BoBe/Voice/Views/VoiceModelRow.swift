import SwiftUI

struct VoiceModelRow: View {
    enum Role {
        case voice
        case hearing
    }

    let role: Role
    let status: VoiceModelStatus
    @Environment(\.theme) private var theme

    var body: some View {
        HStack(spacing: 12) {
            BobeIconTile(
                systemName: self.role == .voice ? "waveform" : "ear",
                tint: self.role == .voice ? self.theme.colors.primary : self.theme.colors.secondary,
                size: 34
            )
            VStack(alignment: .leading, spacing: 2) {
                Text(self.title)
                    .bobeTextStyle(.rowTitle)
                    .foregroundStyle(self.theme.colors.text)
                Text(self.statusText)
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .contentTransition(.opacity)
            }
            Spacer()
            if case let .downloading(percent?, _, _) = self.status {
                Text("\(percent)%")
                    .bobeTextStyle(.caption)
                    .monospacedDigit()
                    .contentTransition(.numericText())
                    .foregroundStyle(self.theme.colors.textMuted)
            }
            self.statusIcon
                .frame(width: 18, height: 18)
        }
        .padding(.horizontal, 14)
        .frame(minHeight: 58)
        .bobeRaised(cornerRadius: BobeRadius.card)
        .animation(BobeMotion.standard, value: self.status)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(self.title): \(self.statusText)")
    }

    private var title: String {
        L10n.tr(self.role == .voice ? "setup.voice.model.tts" : "setup.voice.model.stt")
    }

    private var statusText: String {
        switch self.status {
        case .installed: L10n.tr("voice.model.status.installed")
        case .downloading: L10n.tr("voice.model.status.downloading")
        case .failed: L10n.tr("voice.model.status.failed")
        case .unknown, .missing: L10n.tr("voice.model.status.pending")
        }
    }

    private var statusIcon: some View {
        Group {
            switch self.status {
            case .installed:
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(self.theme.colors.success)
            case .downloading:
                BobeSpinner(size: 14)
            case .unknown, .missing, .failed:
                Image(systemName: "circle.dotted")
                    .foregroundStyle(self.theme.colors.textMuted.opacity(0.6))
            }
        }
        .accessibilityHidden(true)
    }
}
