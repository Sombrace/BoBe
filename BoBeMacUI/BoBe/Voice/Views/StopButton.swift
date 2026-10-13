import SwiftUI

struct StopButton: View {
    let action: () -> Void
    @Environment(\.theme) private var theme

    var body: some View {
        Button(action: self.action) {
            Image(systemName: "stop.fill")
                .font(.system(size: 8, weight: .bold))
                .foregroundStyle(self.theme.colors.surface)
                .frame(width: 22, height: 22)
                .background(Circle().fill(self.theme.colors.text))
                .contentShape(Circle())
        }
        .buttonStyle(BobePressStyle(scale: 0.88))
        .accessibilityLabel(L10n.tr("menu.voice.stop_speaking"))
        .help(L10n.tr("menu.voice.stop_speaking"))
    }
}
