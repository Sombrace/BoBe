import SwiftUI

@MainActor
@Observable
final class RuntimeRestartRequest {
    private(set) var isRestarting = false
    private(set) var errorMessage: String?
    private let restart: @MainActor () async throws -> Void

    init(restart: @escaping @MainActor () async throws -> Void = { try await BackendService.shared.userRestart() }) {
        self.restart = restart
    }

    func perform() async -> Bool {
        guard !self.isRestarting else { return false }
        self.isRestarting = true
        self.errorMessage = nil
        defer { self.isRestarting = false }
        do {
            try await self.restart()
            return true
        } catch {
            self.errorMessage = L10n.tr("settings.shared.restart_banner.failed")
            return false
        }
    }
}

struct RestartRequiredBanner: View {
    let fields: Set<String>
    let onDismiss: () -> Void
    @State private var request = RuntimeRestartRequest()
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

                if let errorMessage = self.request.errorMessage {
                    Label(errorMessage, systemImage: "exclamationmark.triangle.fill")
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.error)
                        .fixedSize(horizontal: false, vertical: true)
                        .transition(.bobeRise)
                }
            }

            Spacer(minLength: 8)

            if DaemonConfig.endpoint.canRestartRuntime {
                Button {
                    Task {
                        if await self.request.perform() {
                            self.onDismiss()
                        }
                    }
                } label: {
                    HStack(spacing: 5) {
                        if self.request.isRestarting {
                            BobeSpinner(size: 10)
                        }
                        Text(L10n.tr("settings.shared.restart_banner.restart_now"))
                    }
                }
                .bobeButton(.primary, size: .mini)
                .disabled(self.request.isRestarting)
            }

            Button(L10n.tr("settings.shared.restart_banner.dismiss"), action: self.onDismiss)
                .bobeButton(.ghost, size: .mini)
                .disabled(self.request.isRestarting)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .bobeRaised(cornerRadius: 18)
        .animation(BobeMotion.standard, value: self.request.errorMessage)
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
