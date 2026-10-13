import SwiftUI

struct RestartRequiredBanner: View {
    let fields: Set<String>
    let onDismiss: () -> Void
    @Environment(\.theme) private var theme

    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            BobeIconTile(systemName: "arrow.triangle.2.circlepath", tint: self.theme.colors.warning, size: 26)

            VStack(alignment: .leading, spacing: 4) {
                Text(L10n.tr(
                    DaemonConfig.endpoint.canRestartRuntime
                        ? "settings.shared.restart_banner.message"
                        : "settings.providers.restart.external"
                ))
                    .bobeTextStyle(.helper)
                    .fontWeight(.medium)
                    .foregroundStyle(self.theme.colors.text)

                if !self.fields.isEmpty {
                    Text(self.fields.sorted().joined(separator: ", "))
                        .font(.system(size: 11, design: .monospaced))
                        .foregroundStyle(self.theme.colors.textMuted)
                        .lineLimit(2)
                }
            }

            Spacer(minLength: 8)

            if DaemonConfig.endpoint.canRestartRuntime {
                Button(L10n.tr("settings.shared.restart_banner.restart_now")) {
                    Task {
                        do {
                            try await BackendService.shared.userRestart()
                            self.onDismiss()
                        } catch {
                        }
                    }
                }
                .bobeButton(.primary, size: .mini)
            }

            Button(L10n.tr("settings.shared.restart_banner.dismiss"), action: self.onDismiss)
                .bobeButton(.ghost, size: .mini)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .bobeRaised(cornerRadius: 18)
        .transition(.bobeRise)
    }
}

#if !SPM_BUILD
    #Preview("RestartRequiredBanner") {
        RestartRequiredBanner(
            fields: ["example_static_field"],
            onDismiss: {}
        )
        .environment(\.theme, allThemes[0])
        .padding()
        .frame(width: 480)
    }
#endif
