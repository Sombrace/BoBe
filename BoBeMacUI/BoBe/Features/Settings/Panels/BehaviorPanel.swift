import CoreGraphics
import SwiftUI

struct BehaviorPanel: View {
    @Environment(SettingsStore.self) private var store
    @State private var newCheckinTime: Date = Calendar.current.date(
        bySettingHour: 9, minute: 0, second: 0, of: .now
    ) ?? .now
    @Environment(\.theme) private var theme

    private static let timeFormatter: DateFormatter = {
        let formatter = DateFormatter()
        formatter.dateFormat = "HH:mm"
        formatter.locale = Locale(identifier: "en_US_POSIX")
        return formatter
    }()

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                if !self.store.restartFields.isEmpty {
                    RestartRequiredBanner(
                        fields: self.store.restartFields,
                        onDismiss: { self.store.clearRestartFields() }
                    )
                }

                if let error = self.store.error {
                    SettingsErrorBanner(message: error)
                }

                if let settings = self.store.settings {
                    PresenceDial(
                        level: Self.level(for: settings),
                        onSelect: self.apply
                    )
                    if let savedMessage = self.store.savedMessage {
                        SettingsSavedToast(message: savedMessage)
                            .padding(.horizontal, 6)
                    }
                    self.captureSection
                    self.checkinSection
                    self.conversationSection
                    self.goalsSection
                } else if self.store.isLoading {
                    HStack(spacing: 8) {
                        BobeSpinner(size: 14)
                        Text(L10n.tr("settings.behavior.loading"))
                            .bobeTextStyle(.settingsBody)
                            .foregroundStyle(self.theme.colors.textMuted)
                    }
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 40)
                }
            }
            .frame(maxWidth: BobeMetrics.pageMaxWidth, alignment: .leading)
            .padding(24)
            .frame(maxWidth: .infinity)
            .animation(BobeMotion.standard, value: self.store.savedMessage)
        }
        .task { await self.store.refresh() }
        .onDisappear { self.store.cancelToast() }
    }

    static func level(for settings: DaemonSettings) -> ProactivityLevel? {
        switch (settings.captureEnabled, settings.checkinEnabled) {
        case (false, false): .quiet
        case (true, false): .balanced
        case (true, true): .proactive
        case (false, true): nil
        }
    }

    private func apply(_ level: ProactivityLevel) {
        let target = level.settings
        self.store.update {
            $0.captureEnabled = target.captureEnabled
            $0.captureIntervalSeconds = target.captureIntervalSeconds
            $0.checkinEnabled = target.checkinEnabled
        }
    }

    private var captureSection: some View {
        CollapsibleSection(
            title: L10n.tr("settings.behavior.capture.title"),
            icon: "viewfinder",
            description: L10n.tr("settings.behavior.capture.description"),
            toggleBinding: Binding(
                get: { self.store.settings?.captureEnabled ?? false },
                set: { enabled in self.store.update { $0.captureEnabled = enabled } }
            ),
            initiallyExpanded: false
        ) {
            SettingsRow(label: L10n.tr("settings.behavior.capture.interval"), suffix: L10n.tr("settings.units.seconds")) {
                DebouncedNumberInput(
                    value: self.store.binding(\.captureIntervalSeconds, fallback: 120),
                    accessibilityLabel: L10n.tr("settings.behavior.capture.interval"),
                    range: 1 ... 600
                )
            }

            if !CGPreflightScreenCaptureAccess() {
                HStack(spacing: 6) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(self.theme.colors.warning)
                    Text(L10n.tr("settings.behavior.capture.permission_missing"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.warning)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 8)
                    Button(L10n.tr("setup.permissions.open_settings")) {
                        if let url = URL(string: "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_ScreenCapture") {
                            NSWorkspace.shared.open(url)
                        }
                    }
                    .bobeButton(.secondary, size: .mini)
                }
            }
        }
    }

    private var checkinSection: some View {
        CollapsibleSection(
            title: L10n.tr("settings.behavior.checkins.title"),
            icon: "clock.fill",
            description: L10n.tr("settings.behavior.checkins.description"),
            toggleBinding: self.store.binding(\.checkinEnabled, fallback: false),
            initiallyExpanded: false
        ) {
            SettingsRow(label: L10n.tr("settings.behavior.checkins.schedule")) {
                EmptyView()
            }
            FlowLayout(spacing: 6) {
                ForEach(self.store.settings?.checkinTimes ?? [], id: \.self) { time in
                    HStack(spacing: 5) {
                        Image(systemName: "clock")
                            .font(.system(size: 10, weight: .semibold))
                            .foregroundStyle(self.theme.colors.primary)
                        Text(time)
                            .font(.system(size: 12.5, weight: .medium).monospacedDigit())
                            .foregroundStyle(self.theme.colors.text)
                        Button {
                            self.removeCheckinTime(time)
                        } label: {
                            Image(systemName: "xmark")
                                .font(.system(size: 8, weight: .bold))
                                .foregroundStyle(self.theme.colors.textMuted)
                                .frame(width: 16, height: 16)
                                .contentShape(Circle())
                        }
                        .buttonStyle(BobePressStyle(scale: 0.85))
                        .accessibilityLabel(
                            L10n.tr("settings.behavior.checkins.remove_time_accessibility_format", time)
                        )
                    }
                    .padding(.leading, 10)
                    .padding(.trailing, 5)
                    .frame(height: 28)
                    .bobeWell(in: .bobePill)
                    .transition(.bobePop())
                }
            }
            .animation(BobeMotion.snappy, value: self.store.settings?.checkinTimes)

            HStack(spacing: 6) {
                DatePicker(
                    "",
                    selection: self.$newCheckinTime,
                    displayedComponents: .hourAndMinute
                )
                .labelsHidden()
                .datePickerStyle(.compact)
                .accessibilityLabel(L10n.tr("settings.behavior.checkins.time_placeholder"))
                Button(L10n.tr("settings.behavior.checkins.action.add")) { self.addCheckinTime() }
                    .bobeButton(.secondary, size: .small)
            }

            SettingsRow(label: L10n.tr("settings.behavior.checkins.jitter"), suffix: L10n.tr("settings.units.minutes")) {
                DebouncedNumberInput(
                    value: self.store.binding(\.checkinJitterMinutes, fallback: 0),
                    accessibilityLabel: L10n.tr("settings.behavior.checkins.jitter"),
                    range: 0 ... 30
                )
            }
        }
    }

    private var conversationSection: some View {
        CollapsibleSection(
            title: L10n.tr("settings.behavior.conversation.title"),
            icon: "message.fill",
            description: L10n.tr("settings.behavior.conversation.description"),
            initiallyExpanded: false
        ) {
            SettingsRow(
                label: L10n.tr("settings.behavior.conversation.auto_close_after"),
                description: L10n.tr("settings.behavior.conversation.auto_close.description"),
                suffix: L10n.tr("settings.units.minutes")
            ) {
                DebouncedNumberInput(
                    value: self.store.binding(\.conversationAutoCloseMinutes, fallback: 10),
                    accessibilityLabel: L10n.tr("settings.behavior.conversation.auto_close_after"),
                    range: 1 ... 60
                )
            }

            SettingsRow(
                label: L10n.tr("settings.behavior.conversation.inactivity_timeout"),
                description: L10n.tr("settings.behavior.conversation.inactivity_timeout.description"),
                suffix: L10n.tr("settings.units.seconds")
            ) {
                DebouncedNumberInput(
                    value: self.store.binding(\.conversationInactivityTimeoutSeconds, fallback: 300),
                    accessibilityLabel: L10n.tr("settings.behavior.conversation.inactivity_timeout"),
                    range: 5 ... 600
                )
            }
        }
    }

    private var goalsSection: some View {
        CollapsibleSection(
            title: L10n.tr("settings.behavior.goals.title"),
            icon: "target",
            description: L10n.tr("settings.behavior.goals.description"),
            initiallyExpanded: false
        ) {
            SettingsRow(
                label: L10n.tr("settings.behavior.goals.check_interval"),
                description: L10n.tr("settings.behavior.goals.check_interval.description"),
                suffix: L10n.tr("settings.units.minutes")
            ) {
                DebouncedNumberInput(
                    value: self.store.durationMinutesBinding(
                        \.goalCheckIntervalSeconds,
                        fallbackSeconds: 3600
                    ),
                    accessibilityLabel: L10n.tr("settings.behavior.goals.check_interval"),
                    range: 1 ... 120
                )
            }
        }
    }

    private func addCheckinTime() {
        let formatted = Self.timeFormatter.string(from: self.newCheckinTime)
        guard let settings = self.store.settings,
              !settings.checkinTimes.contains(formatted)
        else { return }
        self.store.update {
            $0.checkinTimes.append(formatted)
        }
    }

    private func removeCheckinTime(_ time: String) {
        self.store.update {
            $0.checkinTimes.removeAll { $0 == time }
        }
    }
}

struct PresenceDial: View {
    let level: ProactivityLevel?
    let onSelect: (ProactivityLevel) -> Void

    @Environment(\.theme) private var theme
    @Namespace private var thumb

    private var mood: BobeMood {
        switch self.level {
        case .quiet: .asleep
        case .balanced, nil: .idle
        case .proactive: .eager
        }
    }

    var body: some View {
        VStack(spacing: 16) {
            HStack(alignment: .center, spacing: 14) {
                BobeCharacter(
                    mood: self.mood,
                    size: 72,
                    palette: self.theme.avatar,
                    wantsAttention: self.level == .proactive
                )
                VStack(alignment: .leading, spacing: 4) {
                    Text(self.level?.title ?? L10n.tr("settings.category.behavior"))
                        .bobeTextStyle(.display)
                        .foregroundStyle(self.theme.colors.text)
                        .id(self.level)
                        .transition(.bobeRise)
                    if let level {
                        Text(level.description)
                            .bobeTextStyle(.settingsBody)
                            .foregroundStyle(self.theme.colors.textMuted)
                            .fixedSize(horizontal: false, vertical: true)
                            .id(level)
                            .transition(.bobeRise)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }

            HStack(spacing: 2) {
                ForEach(ProactivityLevel.allCases, id: \.self) { option in
                    Button {
                        withAnimation(BobeMotion.snappy) { self.onSelect(option) }
                    } label: {
                        Label(option.title, systemImage: option.icon)
                            .font(.system(size: 13, weight: .medium))
                            .foregroundStyle(self.level == option ? self.theme.colors.text : self.theme.colors.textMuted)
                            .frame(maxWidth: .infinity)
                            .frame(height: 34)
                            .background {
                                if self.level == option {
                                    Capsule(style: .circular)
                                        .fill(self.theme.colors.surface)
                                        .shadow(color: self.theme.shadowTint.opacity(0.16), radius: 3, y: 1.5)
                                        .matchedGeometryEffect(id: "thumb", in: self.thumb)
                                }
                            }
                            .contentShape(Capsule(style: .circular))
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("settings.behavior.presence.\(option.rawValue)")
                    .accessibilityAddTraits(self.level == option ? .isSelected : [])
                }
            }
            .padding(3)
            .bobeWell(in: .bobePill)
        }
        .padding(18)
        .bobeRaised(cornerRadius: BobeRadius.card, elevation: .lifted)
        .animation(BobeMotion.standard, value: self.level)
    }
}

struct FlowLayout: Layout {
    let spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let result = self.arrange(proposal: proposal, subviews: subviews)
        return result.size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let result = self.arrange(proposal: proposal, subviews: subviews)
        for (index, subview) in subviews.enumerated() {
            subview.place(at: CGPoint(x: bounds.minX + result.positions[index].x, y: bounds.minY + result.positions[index].y), proposal: .unspecified)
        }
    }

    private func arrange(proposal: ProposedViewSize, subviews: Subviews) -> (size: CGSize, positions: [CGPoint]) {
        let maxWidth = proposal.width ?? .infinity
        var positions: [CGPoint] = []
        var x: CGFloat = 0
        var y: CGFloat = 0
        var rowHeight: CGFloat = 0
        var totalHeight: CGFloat = 0

        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x + size.width > maxWidth, x > 0 {
                x = 0
                y += rowHeight + self.spacing
                rowHeight = 0
            }
            positions.append(CGPoint(x: x, y: y))
            rowHeight = max(rowHeight, size.height)
            x += size.width + self.spacing
            totalHeight = y + rowHeight
        }

        return (CGSize(width: maxWidth, height: totalHeight), positions)
    }
}
