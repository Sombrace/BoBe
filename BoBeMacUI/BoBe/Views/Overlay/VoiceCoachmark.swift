import SwiftUI

struct VoiceCoachmark: View {
    @Binding var isVisible: Bool

    @Environment(\.theme) private var theme

    var body: some View {
        if self.isVisible {
            HStack(alignment: .top, spacing: 10) {
                BobeIconTile(systemName: "waveform", tint: self.theme.colors.primary, size: 26)
                VStack(alignment: .leading, spacing: 2) {
                    Text(L10n.tr("overlay.voice.coachmark.title"))
                        .font(.system(size: 12.5, weight: .semibold))
                        .foregroundStyle(self.theme.colors.text)
                    Text(L10n.tr("overlay.voice.coachmark.body"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .frame(maxWidth: 210, alignment: .leading)
            }
            .padding(10)
            .bobeRaised(cornerRadius: 18, elevation: .lifted)
            .transition(.bobeRise)
            .accessibilityElement(children: .combine)
        }
    }
}

enum VoiceCoachmarkFlag {
    private static let key = "bobe.voice.coachmark.seen"

    static var hasSeen: Bool {
        UserDefaults.standard.bool(forKey: key)
    }

    static func markSeen() {
        UserDefaults.standard.set(true, forKey: self.key)
    }
}
