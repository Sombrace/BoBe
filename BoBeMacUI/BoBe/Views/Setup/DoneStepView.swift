import SwiftUI

struct DoneStepView: View {
    let firstGoal: String
    let onLaunch: () -> Void
    @Environment(\.theme) private var theme

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Spacer(minLength: 0)

            SetupStepHeader(title: L10n.tr("setup.done.title"))
                .setupStagger(0)

            if !self.firstGoal.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                HStack(spacing: 11) {
                    BobeIconTile(systemName: "target", tint: SettingsCategory.goals.tint, size: 30)
                    Text(L10n.tr("setup.done.first_goal_format", self.firstGoal))
                        .bobeTextStyle(.settingsBody)
                        .foregroundStyle(self.theme.colors.text)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                }
                .padding(12)
                .bobeRaised(cornerRadius: 18)
                .setupStagger(1)
            }

            Label(L10n.tr("setup.done.hint"), systemImage: "cursorarrow.rays")
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.textMuted)
                .setupStagger(2)

            Spacer(minLength: 0)

            Button(action: self.onLaunch) {
                Label(L10n.tr("setup.done.launch"), systemImage: "arrow.right")
                    .labelStyle(TrailingIconLabelStyle())
            }
            .bobeButton(.primary, size: .regular)
            .keyboardShortcut(.defaultAction)
            .frame(maxWidth: .infinity, alignment: .trailing)
            .setupStagger(3)
        }
    }
}

extension SettingsUpdateRequest {
    init(
        captureEnabled: Bool? = nil,
        captureIntervalSeconds: Int? = nil,
        checkinEnabled: Bool? = nil
    ) {
        self.init(
            bodyTheme: nil,
            captureEnabled: captureEnabled,
            captureIntervalSeconds: captureIntervalSeconds,
            checkinEnabled: checkinEnabled,
            checkinTimes: nil,
            checkinJitterMinutes: nil,
            conversationInactivityTimeoutSeconds: nil,
            conversationAutoCloseMinutes: nil,
            goalCheckIntervalSeconds: nil,
            mcpEnabled: nil,
            voiceEnabled: nil,
            voicePersona: nil,
            voiceSpeed: nil,
            voiceSttLanguage: nil,
            voicePauseSensitivity: nil,
            voiceShowPartialCaption: nil
        )
    }
}
