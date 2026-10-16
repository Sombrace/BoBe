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
        ViewThatFits(in: .horizontal) {
            self.inlineLayout
            self.stackedLayout
        }
        .padding(.leading, 9)
        .padding(.trailing, 8)
        .padding(.vertical, 8)
        .bobeRaised(cornerRadius: 18)
    }

    private var inlineLayout: some View {
        HStack(alignment: .center, spacing: 10) {
            self.icon
            self.text
                .frame(maxWidth: .infinity, alignment: .leading)
            ForEach(self.actions) { action in
                self.button(for: action)
            }
            self.dismissButton
        }
    }

    private var stackedLayout: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 10) {
                self.icon
                self.text
                    .frame(maxWidth: .infinity, alignment: .leading)
                self.dismissButton
            }
            if !self.actions.isEmpty {
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 6) {
                        ForEach(self.actions) { action in
                            self.button(for: action)
                        }
                    }
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(self.actions) { action in
                            self.button(for: action)
                        }
                    }
                }
                .padding(.leading, 36)
            }
        }
    }

    private var icon: some View {
        BobeIconTile(systemName: self.systemName, tint: self.tint, size: 26)
    }

    private var text: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(self.message)
                .font(.system(size: 12.5, weight: .medium))
                .foregroundStyle(self.theme.colors.text)
                .lineLimit(4)
                .fixedSize(horizontal: false, vertical: true)
            if let detail {
                Text(detail)
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .lineLimit(3)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func button(for action: OverlayNoticeAction) -> some View {
        Button(action.title, action: action.perform)
            .buttonStyle(NoticeButtonStyle(isProminent: action.isProminent, tint: self.tint))
    }

    @ViewBuilder
    private var dismissButton: some View {
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
}

private struct NoticeButtonStyle: ButtonStyle {
    let isProminent: Bool
    let tint: Color
    @Environment(\.theme) private var theme

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 12, weight: .semibold))
            .multilineTextAlignment(.center)
            .fixedSize(horizontal: false, vertical: true)
            .foregroundStyle(self.isProminent ? self.theme.onAccent : self.tint)
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .frame(minHeight: 26)
            .background(
                Capsule().fill(self.isProminent ? self.tint : self.tint.opacity(0.12))
            )
            .scaleEffect(configuration.isPressed ? 0.95 : 1)
            .animation(BobeMotion.snappy, value: configuration.isPressed)
    }
}
