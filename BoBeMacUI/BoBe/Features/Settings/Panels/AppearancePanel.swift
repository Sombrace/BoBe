import AppKit
import SwiftUI

struct AppearancePanel: View {
    @Environment(ThemeStore.self) private var themeStore
    @Environment(SettingsStore.self) private var settingsStore
    @Environment(\.theme) private var theme

    private var selectableThemes: [ThemeConfig] {
        [systemTheme()] + allThemes
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 30) {
                self.appThemeSection
                self.physicalBodySection
            }
            .frame(maxWidth: BobeMetrics.pageMaxWidth + 80, alignment: .leading)
            .padding(24)
            .frame(maxWidth: .infinity)
        }
        .task {
            await self.settingsStore.loadIfNeeded(force: true)
            while !Task.isCancelled {
                await self.settingsStore.refreshBodyThemeStatus()
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }

    private var appThemeSection: some View {
        VStack(alignment: .leading, spacing: 14) {
            SettingsSectionHeader(
                icon: "paintpalette.fill",
                title: L10n.tr("settings.appearance.theme.title"),
                description: L10n.tr("settings.appearance.theme.description"),
                tint: SettingsCategory.appearance.tint
            )

            LazyVGrid(columns: self.gridColumns, spacing: 14) {
                ForEach(self.selectableThemes) { themeConfig in
                    ThemeCard(
                        themeConfig: themeConfig,
                        isSelected: themeConfig.themeId == self.themeStore.themeId,
                        onSelect: {
                            withAnimation(BobeMotion.soft) {
                                self.themeStore.setTheme(themeConfig.themeId)
                            }
                        }
                    )
                }
            }
        }
    }

    private var physicalBodySection: some View {
        VStack(alignment: .leading, spacing: 14) {
            SettingsSectionHeader(
                icon: "cube.fill",
                title: L10n.tr("settings.appearance.body.title"),
                description: L10n.tr("settings.appearance.body.description"),
                tint: Color(rgb: 0xE0A22E)
            )

            if let settings = settingsStore.settings {
                self.physicalBodyStatus(settings: settings)

                LazyVGrid(columns: self.physicalBodyGridColumns, spacing: 12) {
                    ForEach(PhysicalBodyTheme.allCases) { bodyTheme in
                        let selected = bodyTheme == settings.bodyTheme
                        PhysicalBodyThemeCard(
                            bodyTheme: bodyTheme,
                            isSelected: selected,
                            saveState: selected ? self.settingsStore.bodyThemeSaveState : .idle,
                            isDisabled: self.settingsStore.bodyThemeSaveState == .applying,
                            onSelect: {
                                Task { await self.settingsStore.setBodyTheme(bodyTheme) }
                            }
                        )
                    }
                }
            } else if self.settingsStore.isLoading {
                HStack(spacing: 8) {
                    BobeSpinner(size: 14)
                    Text(L10n.tr("settings.appearance.body.loading"))
                        .bobeTextStyle(.settingsBody)
                        .foregroundStyle(self.theme.colors.textMuted)
                }
            } else {
                VStack(alignment: .leading, spacing: 8) {
                    Text(
                        self.settingsStore.error
                            ?? L10n.tr("settings.appearance.body.load_error")
                    )
                    .bobeTextStyle(.settingsBody)
                    .foregroundStyle(self.theme.colors.error)

                    Button(L10n.tr("app.common.retry")) {
                        Task { await self.settingsStore.loadIfNeeded(force: true) }
                    }
                    .bobeButton(.secondary, size: .small)
                }
            }
        }
    }

    private var gridColumns: [GridItem] {
        [GridItem(.adaptive(minimum: 150, maximum: 220), spacing: 14)]
    }

    private var physicalBodyGridColumns: [GridItem] {
        [GridItem(.adaptive(minimum: 124, maximum: 170), spacing: 12)]
    }

    @ViewBuilder
    private func physicalBodyStatus(settings: DaemonSettings) -> some View {
        let presentation = self.bodyStatusPresentation(settings: settings)
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                if self.settingsStore.bodyThemeSaveState == .applying {
                    BobeSpinner(size: 12)
                } else {
                    Image(systemName: presentation.icon)
                        .font(.system(size: 11, weight: .semibold))
                }
                Text(presentation.text)
                    .bobeTextStyle(.helper)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .accessibilityElement(children: .combine)
            .accessibilityLabel(presentation.text)

            Spacer(minLength: 0)
            if case .failed = self.settingsStore.bodyThemeSaveState {
                Button(L10n.tr("app.common.retry")) {
                    Task {
                        await self.settingsStore.setBodyTheme(settings.bodyTheme)
                    }
                }
                .bobeButton(.ghost, size: .small)
                .accessibilityLabel(L10n.tr("app.common.retry"))
            }
        }
        .foregroundStyle(presentation.color)
    }

    private struct BodyStatus {
        let icon: String
        let text: String
        let color: Color
    }

    private func bodyStatusPresentation(settings: DaemonSettings) -> BodyStatus {
        switch self.settingsStore.bodyThemeSaveState {
        case .applying:
            BodyStatus(
                icon: "arrow.triangle.2.circlepath",
                text: L10n.tr("settings.appearance.body.status.applying"),
                color: self.theme.colors.textMuted
            )
        case .applied:
            BodyStatus(
                icon: "checkmark.circle.fill",
                text: L10n.tr("settings.appearance.body.status.applied"),
                color: self.theme.colors.success
            )
        case .savedOffline:
            BodyStatus(
                icon: "wifi.slash",
                text: L10n.tr("settings.appearance.body.status.offline_saved"),
                color: self.theme.colors.warning
            )
        case let .failed(message):
            BodyStatus(icon: "exclamationmark.triangle.fill", text: message, color: self.theme.colors.error)
        case .idle where settings.bodyConnected:
            BodyStatus(
                icon: "dot.radiowaves.left.and.right",
                text: L10n.tr("settings.appearance.body.status.connected"),
                color: self.theme.colors.success
            )
        case .idle:
            BodyStatus(
                icon: "wifi.slash",
                text: L10n.tr("settings.appearance.body.status.offline"),
                color: self.theme.colors.textMuted
            )
        }
    }
}

struct ThemeCard: View {
    let themeConfig: ThemeConfig
    let isSelected: Bool
    let onSelect: () -> Void
    @State private var isHovered = false

    private var shape: RoundedRectangle {
        RoundedRectangle(cornerRadius: BobeRadius.card, style: .continuous)
    }

    var body: some View {
        Button(action: self.onSelect) {
            VStack(spacing: 10) {
                BobeCharacter(
                    mood: self.isSelected ? .happy : (self.isHovered ? .idle : .asleep),
                    size: 64,
                    palette: self.themeConfig.avatar,
                    isLively: self.isHovered || self.isSelected
                )
                .frame(height: 84)

                VStack(spacing: 3) {
                    Text(self.themeConfig.name)
                        .font(.system(size: 13, weight: .semibold, design: .rounded))
                        .foregroundStyle(self.themeConfig.colors.text)
                    HStack(spacing: 4) {
                        ForEach(
                            [self.themeConfig.colors.primary, self.themeConfig.colors.secondary, self.themeConfig.colors.tertiary],
                            id: \.self
                        ) { swatch in
                            Circle().fill(swatch).frame(width: 7, height: 7)
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 16)
            .background(self.shape.fill(self.themeConfig.colors.background))
            .overlay {
                self.shape.strokeBorder(
                    self.isSelected ? self.themeConfig.colors.primary : self.themeConfig.colors.border.opacity(0.7),
                    lineWidth: self.isSelected ? 2 : 1
                )
            }
            .overlay(alignment: .topTrailing) {
                if self.isSelected {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: 16))
                        .foregroundStyle(self.themeConfig.colors.primary)
                        .padding(9)
                        .transition(.bobePop())
                }
            }
            .shadow(
                color: .black.opacity(self.isHovered ? 0.14 : 0.06),
                radius: self.isHovered ? 12 : 4,
                y: self.isHovered ? 7 : 2
            )
            .offset(y: self.isHovered ? -2 : 0)
        }
        .buttonStyle(BobePressStyle(scale: 0.97))
        .accessibilityLabel(L10n.tr("settings.appearance.theme.accessibility_format", self.themeConfig.name))
        .accessibilityAddTraits(self.isSelected ? .isSelected : [])
        .help(self.themeConfig.description)
        .animation(BobeMotion.standard, value: self.isHovered)
        .animation(BobeMotion.standard, value: self.isSelected)
        .onHover { self.isHovered = $0 }
    }
}

private struct PhysicalBodyThemeCard: View {
    let bodyTheme: PhysicalBodyTheme
    let isSelected: Bool
    let saveState: BodyThemeSaveState
    let isDisabled: Bool
    let onSelect: () -> Void

    @Environment(\.theme) private var theme
    @State private var isHovered = false

    var body: some View {
        Button(action: self.onSelect) {
            VStack(alignment: .leading, spacing: 8) {
                PhysicalBobePreview(bodyTheme: self.bodyTheme)
                    .frame(maxWidth: .infinity)
                    .frame(height: 86)
                    .accessibilityHidden(true)

                HStack(spacing: 5) {
                    Text(self.bodyTheme.name)
                        .bobeTextStyle(.rowMeta)
                        .fontWeight(.semibold)
                        .foregroundStyle(self.theme.colors.text)
                        .lineLimit(2)
                    Spacer(minLength: 0)
                }

                if self.isSelected {
                    HStack(spacing: 5) {
                        self.selectionIndicator
                            .font(.system(size: 9, weight: .semibold))
                        Text(self.selectionLabel)
                            .font(.system(size: 9, weight: .semibold))
                            .lineLimit(2)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .foregroundStyle(self.selectionColor)
                }
            }
            .frame(maxWidth: .infinity, minHeight: 138, alignment: .topLeading)
            .padding(10)
            .background(
                RoundedRectangle(cornerRadius: 12)
                    .fill(self.theme.colors.surface)
                    .shadow(color: .black.opacity(0.05), radius: 4, y: 2)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 12)
                    .stroke(
                        self.isSelected ? self.theme.colors.primary : self.theme.colors.border,
                        lineWidth: self.isSelected ? 2 : 1
                    )
            )
        }
        .buttonStyle(.plain)
        .disabled(self.isDisabled)
        .accessibilityLabel(self.cardAccessibilityLabel)
        .accessibilityHint(L10n.tr("settings.appearance.body.theme.accessibility_hint"))
        .accessibilityAddTraits(self.isSelected ? .isSelected : [])
        .scaleEffect(BobeMotion.hoverScale(self.isHovered && !self.isDisabled))
        .animation(BobeMotion.hover, value: self.isHovered)
        .onHover { self.isHovered = $0 }
    }

    @ViewBuilder
    private var selectionIndicator: some View {
        switch self.saveState {
        case .applying:
            BobeSpinner(size: 12)
        case .savedOffline:
            Image(systemName: "wifi.slash")
                .foregroundStyle(self.theme.colors.warning)
        case .failed:
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(self.theme.colors.error)
        case .idle, .applied:
            Image(systemName: "checkmark")
                .foregroundStyle(self.theme.colors.primary)
        }
    }

    private var selectionLabel: String {
        switch self.saveState {
        case .applying:
            L10n.tr("settings.appearance.body.card.applying")
        case .applied:
            L10n.tr("settings.appearance.body.card.applied")
        case .savedOffline:
            L10n.tr("settings.appearance.body.card.offline_saved")
        case .failed:
            L10n.tr("settings.appearance.body.card.failed")
        case .idle:
            L10n.tr("settings.appearance.body.card.selected")
        }
    }

    private var selectionColor: Color {
        switch self.saveState {
        case .savedOffline:
            self.theme.colors.warning
        case .failed:
            self.theme.colors.error
        case .idle, .applying, .applied:
            self.theme.colors.primary
        }
    }

    private var cardAccessibilityLabel: String {
        let label = L10n.tr(
            "settings.appearance.body.theme.accessibility_format",
            self.bodyTheme.name
        )
        return self.isSelected ? "\(label), \(self.selectionLabel)" : label
    }
}

private struct PhysicalBobePreview: View {
    let bodyTheme: PhysicalBodyTheme

    @Environment(\.theme) private var theme

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 12)
                .fill(self.theme.colors.text.opacity(0.16))

            if let previewImage {
                Image(nsImage: previewImage)
                    .resizable()
                    .interpolation(.medium)
                    .aspectRatio(280 / 240, contentMode: .fit)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
                    .padding(4)
            } else {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(self.theme.colors.error)
            }
        }
        .overlay {
            RoundedRectangle(cornerRadius: 12)
                .stroke(self.theme.colors.border, lineWidth: 1)
        }
    }

    private var previewImage: NSImage? {
        guard let url = Bundle.appResources.url(
            forResource: "theme-\(self.bodyTheme.rawValue)",
            withExtension: "png"
        ) else {
            return nil
        }
        return NSImage(contentsOf: url)
    }
}

#if !SPM_BUILD
    #Preview("Appearance Panel") {
        AppearancePanel()
            .environment(\.theme, allThemes[0])
            .environment(ThemeStore.shared)
            .frame(width: 600, height: 500)
    }

    #Preview("Theme Card") {
        HStack(spacing: 16) {
            ThemeCard(themeConfig: allThemes[0], isSelected: true, onSelect: {})
            ThemeCard(themeConfig: allThemes[1], isSelected: false, onSelect: {})
        }
        .padding()
        .frame(width: 500)
    }
#endif
