import SwiftUI

struct EngineChoiceStepView: View {
    @Binding var selection: EngineChoice?
    let onContinue: () -> Void

    @Environment(\.theme) private var theme
    @FocusState private var focused: EngineChoice?

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            SetupStepHeader(title: L10n.tr("setup.engine.title"))
                .padding(.top, 8)

            VStack(spacing: 10) {
                SetupChoiceCard(
                    icon: "cloud.fill",
                    tint: SettingsCategory.userProfiles.tint,
                    title: L10n.tr("setup.engine.provider.title"),
                    subtitle: L10n.tr("setup.engine.provider.subtitle"),
                    isSelected: self.selection == .provider,
                    action: { self.selection = .provider }
                )
                .focused(self.$focused, equals: .provider)
                .setupStagger(0)

                SetupChoiceCard(
                    icon: "macbook",
                    tint: SettingsCategory.goals.tint,
                    title: L10n.tr("setup.engine.local.title"),
                    subtitle: DaemonConfig.endpoint.managesLocalProcess
                        ? L10n.tr("setup.engine.local.subtitle")
                        : L10n.tr("setup.engine.local.remote_subtitle"),
                    isSelected: self.selection == .local,
                    action: { self.selection = .local },
                    detail: { self.bestFor(L10n.tr("setup.engine.local.best_for")) }
                )
                .focused(self.$focused, equals: .local)
                .setupStagger(1)
            }
            .onKeyPress(.upArrow) {
                self.selection = .provider
                self.focused = self.selection
                return .handled
            }
            .onKeyPress(.downArrow) {
                self.selection = .local
                self.focused = self.selection
                return .handled
            }

            Spacer(minLength: 0)

            HStack {
                Spacer()
                Button(L10n.tr("setup.welcome.continue"), action: self.onContinue)
                    .bobeButton(.primary, size: .regular)
                    .keyboardShortcut(.defaultAction)
                    .disabled(self.selection == nil)
            }
        }
    }

    private func bestFor(_ text: String) -> some View {
        Text(text)
            .bobeTextStyle(.caption)
            .foregroundStyle(self.theme.colors.primary)
            .padding(.horizontal, 8)
            .frame(minHeight: 20)
            .background(Capsule(style: .circular).fill(self.theme.accentSoft))
            .padding(.top, 3)
    }
}
