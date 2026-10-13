import SwiftUI

struct OverlayNoticeAction: Identifiable {
    let id: String
    let title: String
    var isProminent = false
    let perform: () -> Void
}

struct OverlayNotice: View {
    enum Tone {
        case info
        case caution
        case danger
    }

    let systemName: String
    let tone: Tone
    let message: String
    var detail: String?
    var actions: [OverlayNoticeAction] = []
    var onDismiss: (() -> Void)?

    @Environment(\.theme) private var theme

    private var tint: Color {
        switch self.tone {
        case .info: self.theme.colors.tertiary
        case .caution: self.theme.colors.warning
        case .danger: self.theme.colors.error
        }
    }

    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            BobeIconTile(systemName: self.systemName, tint: self.tint, size: 26)
            VStack(alignment: .leading, spacing: 2) {
                Text(self.message)
                    .font(.system(size: 12.5, weight: .medium))
                    .foregroundStyle(self.theme.colors.text)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                if let detail {
                    Text(detail)
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                        .lineLimit(3)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            ForEach(self.actions) { action in
                Button(action.title, action: action.perform)
                    .buttonStyle(NoticeButtonStyle(isProminent: action.isProminent, tint: self.tint))
            }
            if let onDismiss {
                Button(action: onDismiss) {
                    Image(systemName: "xmark")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(self.theme.colors.textMuted)
                        .frame(width: 22, height: 22)
                        .contentShape(Circle())
                }
                .buttonStyle(BobePressStyle(scale: 0.88))
                .accessibilityLabel(L10n.tr("overlay.floating_bubble.dismiss"))
            }
        }
        .padding(.leading, 9)
        .padding(.trailing, 8)
        .padding(.vertical, 8)
        .bobeRaised(cornerRadius: 18)
    }
}

private struct NoticeButtonStyle: ButtonStyle {
    let isProminent: Bool
    let tint: Color
    @Environment(\.theme) private var theme

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 12, weight: .semibold))
            .foregroundStyle(self.isProminent ? self.theme.onAccent : self.tint)
            .padding(.horizontal, 10)
            .frame(height: 26)
            .background(
                Capsule().fill(self.isProminent ? self.tint : self.tint.opacity(0.12))
            )
            .scaleEffect(configuration.isPressed ? 0.95 : 1)
            .animation(BobeMotion.snappy, value: configuration.isPressed)
    }
}
