import SwiftUI

struct PersonalizeStepView: View {
    @Binding var preferredName: String
    @Binding var firstGoal: String
    let onContinue: () -> Void

    @Environment(\.theme) private var theme
    @FocusState private var focusedField: Field?

    private enum Field {
        case name
        case goal
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 22) {
            SetupStepHeader(title: L10n.tr("setup.personalize.title"))
                .padding(.top, 8)

            self.field(
                icon: "person.fill",
                tint: SettingsCategory.userProfiles.tint,
                label: L10n.tr("setup.personalize.name.label")
            ) {
                BobeTextField(
                    placeholder: L10n.tr("setup.personalize.name.placeholder"),
                    text: self.$preferredName,
                    focused: Binding(
                        get: { self.focusedField == .name },
                        set: { if $0 { self.focusedField = .name } }
                    )
                )
                .accessibilityIdentifier("setup.personalize.name")
            }
            .setupStagger(0)

            self.field(
                icon: "target",
                tint: SettingsCategory.goals.tint,
                label: L10n.tr("setup.personalize.goal.label")
            ) {
                BobeTextField(
                    placeholder: L10n.tr("setup.personalize.goal.placeholder"),
                    text: self.$firstGoal,
                    onSubmit: self.onContinue
                )
                .accessibilityIdentifier("setup.personalize.goal")
            }
            .setupStagger(1)

            Spacer(minLength: 0)

            HStack {
                Text(L10n.tr("setup.personalize.optional"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                Spacer()
                Button(L10n.tr("setup.welcome.continue"), action: self.onContinue)
                    .bobeButton(.primary)
                    .keyboardShortcut(.defaultAction)
                    .accessibilityIdentifier("setup.personalize.continue")
            }
        }
        .onAppear { self.focusedField = .name }
    }

    private func field(
        icon: String,
        tint: Color,
        label: String,
        @ViewBuilder input: () -> some View
    ) -> some View {
        HStack(alignment: .top, spacing: 13) {
            BobeIconTile(systemName: icon, tint: tint, size: 34)
            VStack(alignment: .leading, spacing: 7) {
                Text(label)
                    .bobeTextStyle(.heading)
                    .foregroundStyle(self.theme.colors.text)
                input()
            }
        }
    }
}
