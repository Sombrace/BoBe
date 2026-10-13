import SwiftUI

struct ExpertModePanel: View {
    @Environment(\.theme) private var theme
    @Environment(ExpertMode.self) private var expert

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                Text(L10n.tr("settings.expert.description"))
                    .bobeTextStyle(.settingsBody)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .fixedSize(horizontal: false, vertical: true)

                CollapsibleSection(
                    title: L10n.tr("settings.expert.title"),
                    icon: "wrench.and.screwdriver.fill",
                    description: nil
                ) {
                    SettingsRow(
                        label: L10n.tr("settings.expert.toggle"),
                        description: L10n.tr("settings.expert.toggle.description")
                    ) {
                        BobeToggle(isOn: Binding(
                            get: { self.expert.isEnabled },
                            set: { self.expert.setEnabled($0) }
                        ), accessibilityLabel: L10n.tr("settings.expert.toggle"))
                    }
                }

                CollapsibleSection(
                    title: L10n.tr("settings.expert.section.what_changes"),
                    icon: "list.bullet.rectangle",
                    description: nil
                ) {
                    Text(L10n.tr("settings.expert.section.what_changes.list"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                        .fixedSize(horizontal: false, vertical: true)
                        .lineSpacing(4)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(24)
        }
    }
}
