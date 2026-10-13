import SwiftUI

struct VoiceModelRow: View {
    let model: VoiceModelProgress
    @Environment(\.theme) private var theme

    var body: some View {
        HStack(spacing: 12) {
            BobeIconTile(
                systemName: self.model.kind == VoiceWire.modelKindTts ? "waveform" : "ear",
                tint: self.model.kind == VoiceWire.modelKindTts ? self.theme.colors.primary : self.theme.colors.secondary,
                size: 34
            )
            VStack(alignment: .leading, spacing: 2) {
                Text(self.displayName)
                    .bobeTextStyle(.rowTitle)
                    .foregroundStyle(self.theme.colors.text)
                Text(self.localizedStatus)
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .contentTransition(.opacity)
            }
            Spacer()
            if let percent = self.model.percent {
                Text("\(percent)%")
                    .bobeTextStyle(.caption)
                    .monospacedDigit()
                    .contentTransition(.numericText())
                    .foregroundStyle(self.theme.colors.textMuted)
            }
            VoiceModelStatusIcon(status: self.model.status)
                .frame(width: 18, height: 18)
        }
        .padding(.horizontal, 14)
        .frame(minHeight: 58)
        .bobeRaised(cornerRadius: BobeRadius.card)
        .animation(BobeMotion.standard, value: self.model.status)
        .animation(BobeMotion.standard, value: self.model.percent)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(self.displayName): \(self.localizedStatus)")
    }

    private var displayName: String {
        if self.model.kind == VoiceWire.modelKindTts {
            return L10n.tr("setup.voice.model.tts")
        }
        return self.model.label == self.model.kind ? self.model.label.uppercased() : self.model.label
    }

    private var localizedStatus: String {
        switch self.model.status {
        case "installed": L10n.tr("voice.model.status.installed")
        case "already installed": L10n.tr("voice.model.status.already_installed")
        case "downloading": L10n.tr("voice.model.status.downloading")
        case "pending": L10n.tr("voice.model.status.pending")
        case "failed": L10n.tr("voice.model.status.failed")
        case "canceled": L10n.tr("voice.model.status.canceled")
        default: self.model.status
        }
    }
}

struct VoiceModelStatusIcon: View {
    let status: String
    @Environment(\.theme) private var theme

    var body: some View {
        Group {
            switch self.status {
            case "installed", "already installed":
                Image(systemName: "checkmark.circle.fill")
                    .foregroundStyle(self.theme.colors.success)
            case "downloading":
                BobeSpinner(size: 14)
            default:
                Image(systemName: "circle.dotted")
                    .foregroundStyle(self.theme.colors.textMuted.opacity(0.6))
            }
        }
        .accessibilityHidden(true)
    }
}
