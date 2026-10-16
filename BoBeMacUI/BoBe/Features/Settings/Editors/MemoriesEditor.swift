import SwiftUI

let memoryDefaultBody = """
# BoBe Memory

## Profile

## Active Goals

## Long-term

## Recent
"""

private let memoryTargetMaxBytes = 50 * 1024
private let memoryConsolidationDeltaThreshold = 200

struct MemoriesEditor: View {
    @State private var text = ""
    @State private var savedText = ""
    @State private var isLoading = false
    @State private var isSaving = false
    @State private var status: String?
    @State private var statusIsError = false
    @State private var consolidationHintShown = false
    @State private var editCoordinator = SettingsEditCoordinator.shared
    @State private var editorMode: ContextEditorMode = .guided
    @Environment(ExpertMode.self) private var expertMode
    @Environment(\.theme) private var theme

    private var isDirty: Bool {
        self.text != self.savedText
    }

    private var byteProgress: Double {
        let count = self.text.utf8.count
        return Double(count) / Double(memoryTargetMaxBytes)
    }

    private var byteLabel: String {
        let style = ByteCountFormatStyle(
            style: .file,
            allowedUnits: [.kb, .mb]
        )
        let used = self.text.utf8.count.formatted(style)
        let target = memoryTargetMaxBytes.formatted(style)
        return L10n.tr("settings.memory.bytes_format", used, target)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(L10n.tr("settings.memory.title"))
                    .font(.headline)
                    .foregroundStyle(self.theme.colors.text)
                Spacer()
            }
            .padding(.bottom, 4)

            Text(L10n.tr("settings.memory.description"))
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.textMuted)

            if self.expertMode.isEnabled {
                BobeSegmented(
                    selection: self.$editorMode,
                    options: ContextEditorMode.allCases,
                    title: \.label
                )
                .frame(width: 170)
                .accessibilityLabel(L10n.tr("settings.context_editor.mode.accessibility"))
            }

            self.editorBody
            self.byteGauge
            self.actionRow
            self.statusFooter
        }
        .padding(20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .task { await self.load() }
        .onChange(of: self.isDirty, initial: true) { _, dirty in
            self.editCoordinator.register(
                isDirty: dirty,
                save: { await self.save() },
                discard: { self.text = self.savedText }
            )
        }
        .onDisappear { self.editCoordinator.unregister() }
        .onChange(of: self.expertMode.isEnabled) { _, enabled in
            if !enabled {
                self.editorMode = .guided
            }
        }
    }

    private var editorBody: some View {
        Group {
            if self.isLoading, self.savedText.isEmpty {
                HStack(spacing: 8) {
                    BobeSpinner(size: 14)
                    Text(L10n.tr("settings.memories.loading"))
                        .bobeTextStyle(.settingsBody)
                        .foregroundStyle(self.theme.colors.textMuted)
                }
                .frame(maxWidth: .infinity, minHeight: 360, alignment: .center)
            } else {
                if self.expertMode.isEnabled, self.editorMode == .raw {
                    CodeEditor(
                        text: self.$text,
                        accessibilityLabel: L10n.tr(
                            "settings.context_editor.raw.accessibility_format",
                            L10n.tr("settings.memory.title")
                        ),
                        theme: self.theme,
                        fontSize: 13
                    )
                    .frame(minHeight: 360, maxHeight: .infinity)
                    .bobeRaised(cornerRadius: 16)
                } else {
                    GuidedMarkdownEditor(
                        text: self.$text,
                        fallbackTitle: L10n.tr("settings.memory.title")
                    )
                    .frame(minHeight: 360, maxHeight: .infinity)
                }
            }
        }
    }

    private var byteGauge: some View {
        HStack(spacing: 10) {
            BobeLinearProgressBar(progress: self.byteProgress, tint: self.gaugeTint)
                .frame(maxWidth: .infinity)
            Text(self.byteLabel)
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(self.theme.colors.textMuted)
        }
    }

    private var gaugeTint: Color {
        switch self.byteProgress {
        case ..<0.75:
            self.theme.colors.secondary
        case ..<0.92:
            self.theme.colors.tertiary
        default:
            self.theme.colors.primary
        }
    }

    private var actionRow: some View {
        HStack(spacing: 8) {
            Button(L10n.tr("settings.memory.action.reload")) {
                self.editCoordinator.requestTransition { Task { await self.load(force: true) } }
            }
            .bobeButton(.secondary, size: .small)
            .disabled(self.isLoading || self.isSaving)

            Spacer()

            Button(L10n.tr("settings.memory.action.reset_default")) {
                self.text = memoryDefaultBody
            }
            .bobeButton(.secondary, size: .small)
            .disabled(self.isLoading || self.isSaving || self.text == memoryDefaultBody)

            Button(self.isSaving
                ? L10n.tr("settings.shared.action.saving")
                : L10n.tr("settings.shared.action.save")) {
                    Task { _ = await self.save() }
                }
                .bobeButton(.primary, size: .small)
                .disabled(!self.isDirty || self.isSaving)
        }
    }

    @ViewBuilder
    private var statusFooter: some View {
        if let status {
            Text(status)
                .bobeTextStyle(.helper)
                .foregroundStyle(self.statusIsError ? self.theme.colors.primary : self.theme.colors.secondary)
        } else if self.consolidationHintShown {
            HStack(spacing: 4) {
                Image(systemName: "moon.stars.fill")
                    .font(.system(size: 11))
                    .foregroundStyle(self.theme.colors.tertiary)
                Text(L10n.tr("settings.memory.consolidation_hint"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.tertiary)
            }
        }
    }

    private func load(force: Bool = false) async {
        if !force, self.isDirty {
            self.statusIsError = false
            self.status = L10n.tr("settings.memory.discard_to_reload")
            return
        }
        self.isLoading = true
        defer { self.isLoading = false }
        do {
            let resp = try await DaemonClient.shared.getMemory()
            self.text = resp.content
            self.savedText = resp.content
            self.status = nil
            self.statusIsError = false
        } catch {
            self.status = error.localizedDescription
            self.statusIsError = true
        }
    }

    private func save() async -> Bool {
        guard !self.isSaving else { return false }
        self.isSaving = true
        defer { self.isSaving = false }
        let submitted = self.text
        let sentLength = submitted.utf8.count
        let selection = EndpointStore.shared.selection
        let client = DaemonClient(endpoint: selection.endpoint)
        do {
            let resp = try await client.updateMemory(submitted)
            guard EndpointStore.shared.selection == selection else { return false }
            self.savedText = resp.content
            self.text = reconcileSavedDraft(current: self.text, submitted: submitted, saved: resp.content)
            self.statusIsError = false
            self.status = L10n.tr("settings.memory.saved")
            self.consolidationHintShown = abs(resp.bytes - sentLength) > memoryConsolidationDeltaThreshold
            return !self.isDirty
        } catch {
            self.status = error.localizedDescription
            self.statusIsError = true
            return false
        }
    }
}
