import SwiftUI

enum ProactivityLevel: String, CaseIterable, Sendable {
    case quiet
    case balanced
    case proactive

    struct Settings: Sendable {
        let captureEnabled: Bool
        let captureIntervalSeconds: Int
        let checkinEnabled: Bool
    }

    var settings: Settings {
        switch self {
        case .quiet:
            Settings(captureEnabled: false, captureIntervalSeconds: 120, checkinEnabled: false)
        case .balanced:
            Settings(captureEnabled: true, captureIntervalSeconds: 120, checkinEnabled: false)
        case .proactive:
            Settings(captureEnabled: true, captureIntervalSeconds: 45, checkinEnabled: true)
        }
    }

    var title: String {
        L10n.tr("setup.proactivity.\(self.rawValue).title")
    }

    var description: String {
        L10n.tr("setup.proactivity.\(self.rawValue).description")
    }

    var icon: String {
        switch self {
        case .quiet: "moon.stars.fill"
        case .balanced: "circle.lefthalf.filled"
        case .proactive: "sparkles"
        }
    }
}

struct ProactivityStepView: View {
    @Binding var selection: ProactivityLevel
    let onContinue: () -> Void

    @Environment(\.theme) private var theme

    private func capability(_ symbol: String, label: String, enabled: Bool) -> some View {
        Label(label, systemImage: enabled ? symbol : "minus")
            .bobeTextStyle(.caption)
            .foregroundStyle(enabled ? self.theme.colors.primary : self.theme.colors.textMuted.opacity(0.7))
            .help(L10n.tr(enabled ? "settings.providers.on" : "settings.providers.off"))
            .accessibilityValue(L10n.tr(enabled ? "settings.providers.on" : "settings.providers.off"))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            SetupStepHeader(title: L10n.tr("setup.proactivity.title"))
                .padding(.top, 8)

            VStack(spacing: 10) {
                ForEach(Array(ProactivityLevel.allCases.enumerated()), id: \.element) { index, level in
                    SetupChoiceCard(
                        icon: level.icon,
                        tint: SettingsCategory.behavior.tint,
                        title: level.title,
                        subtitle: level.description,
                        isSelected: self.selection == level,
                        action: { self.selection = level },
                        detail: {
                            HStack(spacing: 12) {
                                self.capability("viewfinder", label: L10n.tr("settings.behavior.capture.title"),
                                                enabled: level.settings.captureEnabled)
                                self.capability("clock", label: L10n.tr("settings.behavior.checkins.title"),
                                                enabled: level.settings.checkinEnabled)
                                self.capability("target", label: L10n.tr("settings.providers.quiet_goals"), enabled: true)
                            }
                            .padding(.top, 3)
                        }
                    )
                    .accessibilityIdentifier("setup.proactivity.\(level.rawValue)")
                    .setupStagger(index)
                }
            }

            Spacer(minLength: 0)

            Button(L10n.tr("setup.welcome.continue"), action: self.onContinue)
                .bobeButton(.primary)
                .keyboardShortcut(.defaultAction)
                .accessibilityIdentifier("setup.proactivity.continue")
                .frame(maxWidth: .infinity, alignment: .trailing)
        }
    }
}
