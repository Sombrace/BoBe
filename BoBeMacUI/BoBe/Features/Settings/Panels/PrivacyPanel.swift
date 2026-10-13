import AppKit
import SwiftUI

struct PrivacyPanel: View {
    private let deleteKeyword = "DELETE"

    @State private var showDeleteControls = false
    @State private var deleteConfirmationText = ""
    @State private var isDeletingAll = false
    @State private var statusMessage: String?
    @State private var statusIsError = false
    @State private var diagnosticsEnabled = DiagnosticsMetrics.shared.isEnabled
    @State private var diagnosticsStatusMessage: String?
    @State private var diagnosticsStatusIsError = false
    @Environment(\.theme) private var theme

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                ProviderDataFlow()

                BobeHairline()

                VStack(alignment: .leading, spacing: 12) {
                    HStack(spacing: 8) {
                        Image(systemName: "waveform.path.ecg")
                            .font(.system(size: 16))
                            .foregroundStyle(self.theme.colors.primary)
                        Text(L10n.tr("settings.privacy.diagnostics.title"))
                            .bobeTextStyle(.heading)
                            .foregroundStyle(self.theme.colors.text)
                    }

                    Text(L10n.tr("settings.privacy.diagnostics.description"))
                        .bobeTextStyle(.settingsBody)
                        .foregroundStyle(self.theme.colors.textMuted)

                    Toggle(
                        L10n.tr("settings.privacy.diagnostics.enabled"),
                        isOn: self.$diagnosticsEnabled
                    )
                    .toggleStyle(BobeLabeledSwitchStyle())
                    .accessibilityIdentifier("settings.privacy.diagnostics.enabled")
                    .onChange(of: self.diagnosticsEnabled) { _, enabled in
                        Task { await self.updateDiagnosticsEnabled(enabled) }
                    }

                    Text(L10n.tr("settings.privacy.diagnostics.excluded"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)

                    HStack(spacing: 8) {
                        Button(L10n.tr("settings.privacy.diagnostics.export")) {
                            self.exportDiagnostics()
                        }
                        .bobeButton(.secondary, size: .small)
                        .accessibilityIdentifier("settings.privacy.diagnostics.export")
                        .disabled(!self.diagnosticsEnabled)

                        Button(L10n.tr("settings.privacy.diagnostics.delete")) {
                            self.deleteDiagnostics()
                        }
                        .bobeButton(.secondary, size: .small)
                        .accessibilityIdentifier("settings.privacy.diagnostics.delete")
                    }

                    if let diagnosticsStatusMessage {
                        Text(diagnosticsStatusMessage)
                            .bobeTextStyle(.helper)
                            .accessibilityIdentifier("settings.privacy.diagnostics.status")
                            .foregroundStyle(
                                self.diagnosticsStatusIsError
                                    ? self.theme.colors.error
                                    : self.theme.colors.success
                            )
                    }
                }

                BobeHairline()

                HStack(spacing: 8) {
                    Image(systemName: "externaldrive.fill")
                        .font(.system(size: 16))
                        .foregroundStyle(self.theme.colors.primary)
                    Text(L10n.tr("settings.privacy.storage.title"))
                        .bobeTextStyle(.heading)
                        .foregroundStyle(self.theme.colors.text)
                }

                Text(L10n.tr("settings.privacy.storage.description"))
                    .bobeTextStyle(.settingsBody)
                    .foregroundStyle(self.theme.colors.textMuted)

                VStack(alignment: .leading, spacing: 8) {
                    Text(L10n.tr("settings.privacy.storage.included.title"))
                        .bobeTextStyle(.rowTitle)
                        .foregroundStyle(self.theme.colors.text)
                    Text(L10n.tr("settings.privacy.storage.included.list"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .bobeRaised(cornerRadius: 16)

                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 8) {
                        Image(systemName: "trash.fill")
                            .font(.system(size: 15))
                            .foregroundStyle(self.theme.colors.destructive)
                        Text(L10n.tr("settings.privacy.danger.title"))
                            .bobeTextStyle(.rowTitle)
                            .foregroundStyle(self.theme.colors.text)
                    }

                    Text(L10n.tr("settings.privacy.danger.description"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)

                    if self.showDeleteControls {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(L10n.tr("settings.privacy.danger.confirm_prompt_format", self.deleteKeyword))
                                .bobeTextStyle(.helper)
                                .foregroundStyle(self.theme.colors.textMuted)
                            BobeTextField(placeholder: self.deleteKeyword, text: self.$deleteConfirmationText, width: 220)

                            HStack(spacing: 8) {
                                Button(L10n.tr("settings.editor.action.cancel")) {
                                    self.showDeleteControls = false
                                    self.deleteConfirmationText = ""
                                }
                                .bobeButton(.secondary, size: .small)

                                Button(
                                    self.isDeletingAll
                                        ? L10n.tr("settings.privacy.danger.action.deleting")
                                        : L10n.tr("settings.privacy.danger.action.delete_all")
                                ) {
                                    Task { await self.deleteAllData() }
                                }
                                .bobeButton(.destructive, size: .small)
                                .disabled(self.deleteConfirmationText != self.deleteKeyword || self.isDeletingAll)

                                if self.isDeletingAll {
                                    BobeSpinner(size: 14)
                                }
                            }
                        }
                    } else {
                        Button(L10n.tr("settings.privacy.danger.action.delete_all")) {
                            statusMessage = nil
                            self.showDeleteControls = true
                        }
                        .bobeButton(.destructive, size: .small)
                    }

                    if let statusMessage {
                        Text(statusMessage)
                            .bobeTextStyle(.helper)
                            .foregroundStyle(self.statusIsError ? self.theme.colors.error : self.theme.colors.success)
                    }
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    RoundedRectangle(cornerRadius: 10)
                        .fill(self.theme.colors.surface)
                        .stroke(self.theme.colors.destructive.opacity(0.35), lineWidth: 1)
                )
            }
            .padding(24)
        }
        .accessibilityIdentifier("settings.panel.privacy")
    }

    private func updateDiagnosticsEnabled(_ enabled: Bool) async {
        do {
            try await DiagnosticsMetrics.shared.setEnabled(enabled)
            self.diagnosticsStatusIsError = false
            self.diagnosticsStatusMessage = enabled
                ? L10n.tr("settings.privacy.diagnostics.status.enabled")
                : L10n.tr("settings.privacy.diagnostics.status.disabled")
        } catch {
            self.diagnosticsEnabled = DiagnosticsMetrics.shared.isEnabled
            self.diagnosticsStatusIsError = true
            self.diagnosticsStatusMessage = error.localizedDescription
        }
    }

    private func exportDiagnostics() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.json]
        panel.canCreateDirectories = true
        panel.nameFieldStringValue = "bobe-diagnostics.json"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            try DiagnosticsMetrics.shared.export(to: url)
            self.diagnosticsStatusIsError = false
            self.diagnosticsStatusMessage = L10n.tr("settings.privacy.diagnostics.status.exported")
        } catch {
            self.diagnosticsStatusIsError = true
            self.diagnosticsStatusMessage = error.localizedDescription
        }
    }

    private func deleteDiagnostics() {
        do {
            try DiagnosticsMetrics.shared.deleteRetainedData()
            self.diagnosticsStatusIsError = false
            self.diagnosticsStatusMessage = L10n.tr("settings.privacy.diagnostics.status.deleted")
        } catch {
            self.diagnosticsStatusIsError = true
            self.diagnosticsStatusMessage = error.localizedDescription
        }
    }

    private func deleteAllData() async {
        guard self.deleteConfirmationText == self.deleteKeyword else { return }
        self.isDeletingAll = true
        defer { isDeletingAll = false }

        do {
            let response = try await DaemonClient.shared.purgePersonalData()
            self.showDeleteControls = false
            self.deleteConfirmationText = ""
            self.statusIsError = false
            self.statusMessage = response.resultSummary
        } catch {
            self.statusIsError = true
            self.statusMessage = error.localizedDescription
        }
    }
}

#if !SPM_BUILD
    #Preview("Privacy Panel") {
        PrivacyPanel()
            .environment(\.theme, allThemes[0])
            .environment(SettingsStore.shared)
            .frame(width: 600, height: 500)
    }
#endif
