import SwiftUI

protocol EditableEntity: Identifiable, Sendable where ID: Sendable {
    var name: String { get }
    var content: String { get }
    var enabled: Bool { get }
    var isDefault: Bool { get }
}

extension Soul: EditableEntity {}
extension UserProfile: EditableEntity {}

struct EntityActions<E: EditableEntity, CR: Sendable, UR: Sendable> {
    let list: @Sendable () async throws -> [E]
    let create: @Sendable (CR) async throws -> E
    let update: @Sendable (E.ID, UR) async throws -> E
    let delete: @Sendable (E.ID) async throws -> Void
    let enable: @Sendable (E.ID) async throws -> Void
    let disable: @Sendable (E.ID) async throws -> Void
    let buildCreateRequest: (_ name: String) -> CR
    let buildUpdateRequest: (_ content: String) -> UR
}

struct EntityEditorStrings {
    let paneTitle: String
    let newPlaceholder: String
    let loading: String
    let emptyTitle: String
    let emptyDescription: String
    let emptySelectPrompt: String
    let deleteAccessibility: String
    let toggleAccessibility: String
}

struct EntityListEditor<E: EditableEntity, CR: Sendable, UR: Sendable>: View {
    @State private var entities: [E] = []
    @State private var editorState = SettingsEditorState<E.ID>()
    @State private var editorContent = ""
    @State private var newName = ""
    @State private var editCoordinator = SettingsEditCoordinator.shared
    @State private var editorMode: ContextEditorMode = .guided
    @Environment(ExpertMode.self) private var expertMode
    @Environment(\.theme) private var theme

    let strings: EntityEditorStrings
    let emptyIcon: String
    let actions: EntityActions<E, CR, UR>

    private var selectedEntity: E? {
        self.entities.first { $0.id == self.editorState.selectedId }
    }

    var body: some View {
        SettingsEditorScaffold(hasSelection: self.selectedEntity != nil) {
            self.listPane
        } detailPane: {
            if let entity = self.selectedEntity {
                self.detailPane(for: entity)
            }
        } emptyPane: {
            self.emptyPane
        }
        .onChange(of: self.editorState.selectedId) { _, newId in
            if let entity = self.entities.first(where: { $0.id == newId }) {
                self.editorContent = entity.content
                self.editorState.setDirty(false)
                self.editorState.dismissDeleteConfirmation()
            }
        }
        .task { await self.loadEntities() }
        .onChange(of: self.editorState.isDirty, initial: true) { _, dirty in
            self.registerEditSession(isDirty: dirty)
        }
        .onDisappear { self.editCoordinator.unregister() }
    }


    private var listPane: some View {
        VStack(alignment: .leading, spacing: 0) {
            SettingsPaneHeader(title: self.strings.paneTitle) { self.editorState.isCreating.toggle() }
                .padding(.bottom, 12)

            if self.editorState.isCreating {
                HStack(spacing: 6) {
                    BobeTextField(placeholder: self.strings.newPlaceholder, text: self.$newName) {
                        if !self.newName.isEmpty { self.createEntity() }
                    }
                    Button(L10n.tr("settings.editor.action.create")) { self.createEntity() }
                        .bobeButton(.primary, size: .small)
                        .disabled(self.newName.isEmpty)
                    Button {
                        self.editorState.setCreating(false)
                        self.newName = ""
                    } label: {
                        Text(L10n.tr("settings.editor.action.cancel"))
                    }
                    .bobeButton(.secondary, size: .small)
                }
            }

            if self.editorState.isCreating, let errorMessage = self.editorState.errorMessage {
                SettingsEditorErrorText(message: errorMessage)
            }

            if self.editorState.isLoading, self.entities.isEmpty {
                HStack(spacing: 8) {
                    BobeSpinner(size: 14)
                    Text(self.strings.loading)
                        .bobeTextStyle(.body)
                        .foregroundStyle(self.theme.colors.textMuted)
                }
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.top, 20)
            } else if self.entities.isEmpty, !self.editorState.isLoading {
                VStack(spacing: 8) {
                    Image(systemName: self.emptyIcon)
                        .font(.system(size: 28))
                        .foregroundStyle(self.theme.colors.textMuted)
                    Text(self.strings.emptyTitle)
                        .bobeTextStyle(.rowTitle)
                        .foregroundStyle(self.theme.colors.textMuted)
                    Text(self.strings.emptyDescription)
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted.opacity(0.7))
                        .multilineTextAlignment(.center)
                }
                .frame(maxWidth: .infinity)
                .padding(.top, 32)
            } else {
                ScrollView {
                    LazyVStack(spacing: 4) {
                        ForEach(self.entities) { entity in
                            self.entityRow(entity)
                        }
                    }
                }
                .background(self.theme.colors.background)
            }
        }
    }

    private func entityRow(_ entity: E) -> some View {
        BobeSelectableRow(
            isSelected: self.editorState.selectedId == entity.id,
            content: {
                Button {
                    self.requestSelection(entity.id)
                } label: {
                    HStack {
                        VStack(alignment: .leading) {
                            HStack(spacing: 4) {
                                Text(entity.name)
                                    .bobeTextStyle(.rowTitle)
                                if entity.isDefault {
                                    Text(L10n.tr("settings.editor.badge.default"))
                                        .bobeTextStyle(.badge)
                                        .padding(.horizontal, 4)
                                        .padding(.vertical, 1)
                                        .background(Capsule().fill(self.theme.colors.primary.opacity(0.2)))
                                        .foregroundStyle(self.theme.colors.primary)
                                }
                            }
                            Text(String(entity.content.prefix(60)).replacingOccurrences(of: "\n", with: " "))
                                .bobeTextStyle(.rowMeta)
                                .foregroundStyle(self.theme.colors.textMuted)
                                .lineLimit(1)
                        }
                        Spacer()
                    }
                }
                .buttonStyle(.plain)
                .accessibilityLabel(entity.name)

                BobeToggle(
                    isOn: Binding(
                        get: { entity.enabled },
                        set: { _ in self.toggleEntity(entity) }
                    ),
                    accessibilityLabel: self.strings.toggleAccessibility
                )
            }
        )
    }

    private func detailPane(for entity: E) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Text(entity.name)
                    .bobeTextStyle(.rowTitle)
                    .foregroundStyle(self.theme.colors.text)
                if self.editorState.isDirty {
                    Text(L10n.tr("settings.editor.badge.unsaved"))
                        .bobeTextStyle(.badge)
                        .foregroundStyle(self.theme.colors.tertiary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(self.theme.colors.tertiary.opacity(0.15)))
                }
                if entity.isDefault {
                    Text(L10n.tr("settings.editor.badge.default"))
                        .bobeTextStyle(.badge)
                        .foregroundStyle(self.theme.colors.primary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(Capsule().fill(self.theme.colors.primary.opacity(0.15)))
                }
                Spacer()

                if self.expertMode.isEnabled {
                    BobeSegmented(
                    selection: self.$editorMode,
                    options: ContextEditorMode.allCases,
                    title: \.label
                )
                .frame(width: 170)
                    .accessibilityLabel(L10n.tr("settings.context_editor.mode.accessibility"))
                }

                if !entity.isDefault {
                    self.deleteControls(for: entity)
                }

                SettingsEditorSaveActions(
                    isDirty: self.editorState.isDirty,
                    isSaving: self.editorState.isSaving,
                    onDiscard: self.discardChanges,
                    onSave: { Task { _ = await self.saveEntity() } }
                )
            }

            Group {
                if self.expertMode.isEnabled, self.editorMode == .raw {
                    CodeEditor(
                        text: self.$editorContent,
                        accessibilityLabel: L10n.tr("settings.context_editor.raw.accessibility_format", entity.name),
                        theme: self.theme,
                        fontSize: 13
                    )
                    .bobeRaised(cornerRadius: 16)
                } else {
                    GuidedMarkdownEditor(
                        text: self.$editorContent,
                        fallbackTitle: entity.name
                    )
                }
            }
                .onChange(of: self.editorContent) { _, _ in
                    self.editorState.setDirty(self.editorContent != self.selectedEntity?.content)
                }
                .onChange(of: self.expertMode.isEnabled) { _, enabled in
                    if !enabled {
                        self.editorMode = .guided
                    }
                }

            if let errorMessage = self.editorState.errorMessage {
                SettingsEditorErrorText(message: errorMessage)
            }
        }
    }

    @ViewBuilder
    private func deleteControls(for entity: E) -> some View {
        if self.editorState.showDeleteConfirmation {
            HStack(spacing: 6) {
                Text(L10n.tr("settings.editor.delete.confirm"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.primary)
                Button(L10n.tr("settings.editor.delete.yes")) {
                    self.deleteEntity(entity)
                    self.editorState.dismissDeleteConfirmation()
                }
                .bobeButton(.destructive, size: .small)
                Button(L10n.tr("settings.editor.delete.no")) { self.editorState.dismissDeleteConfirmation() }
                    .bobeButton(.secondary, size: .small)
            }
        } else {
            Button {
                self.editorState.requestDeleteConfirmation()
            } label: {
                Image(systemName: "trash")
            }
            .accessibilityLabel(self.strings.deleteAccessibility)
            .bobeButton(.destructive, size: .small)
        }
    }

    private var emptyPane: some View {
        VStack(spacing: 8) {
            Image(systemName: self.emptyIcon)
                .font(.system(size: 28))
                .foregroundStyle(self.theme.colors.textMuted)
            Text(self.strings.emptySelectPrompt)
                .bobeTextStyle(.rowTitle)
                .foregroundStyle(self.theme.colors.textMuted)
        }
    }


    private func loadEntities() async {
        self.editorState.setLoading(true)
        defer { self.editorState.setLoading(false) }
        do {
            let list = try await self.actions.list()
            self.entities = list
            if self.editorState.selectedId == nil {
                self.editorState.select(self.entities.first?.id)
            }
        } catch {
            self.editorState.setError(error)
        }
    }

    private func createEntity() {
        Task {
            do {
                let request = self.actions.buildCreateRequest(self.newName.lowercased())
                let created = try await self.actions.create(request)
                self.entities.append(created)
                self.newName = ""
                self.editCoordinator.selectCreated(created.id, in: self.$editorState)
            } catch {
                self.editorState.setError(error)
            }
        }
    }

    private func saveEntity() async -> Bool {
        guard let id = self.editorState.selectedId, !self.editorState.isSaving else { return false }
        let submitted = self.editorContent
        let selection = EndpointStore.shared.selection
        self.editorState.setSaving(true)
        defer { self.editorState.setSaving(false) }
        do {
            let request = self.actions.buildUpdateRequest(submitted)
            let updated = try await self.actions.update(id, request)
            guard EndpointStore.shared.selection == selection else { return false }
            if let idx = self.entities.firstIndex(where: { $0.id == id }) {
                self.entities[idx] = updated
            }
            guard self.editorState.selectedId == id else { return false }
            self.editorContent = reconcileSavedDraft(
                current: self.editorContent, submitted: submitted, saved: updated.content
            )
            self.editorState.setDirty(self.editorContent != updated.content)
            return !self.editorState.isDirty
        } catch {
            self.editorState.setError(error)
            return false
        }
    }

    private func deleteEntity(_ entity: E) {
        Task {
            do {
                try await self.actions.delete(entity.id)
                self.entities.removeAll { $0.id == entity.id }
                if self.editorState.selectedId == entity.id {
                    self.editorState.select(self.entities.first?.id)
                }
            } catch {
                self.editorState.setError(error)
            }
        }
    }

    private func toggleEntity(_ entity: E) {
        Task {
            do {
                if entity.enabled {
                    try await self.actions.disable(entity.id)
                } else {
                    try await self.actions.enable(entity.id)
                }
                await self.loadEntities()
            } catch {
                self.editorState.setError(error)
            }
        }
    }

    private func requestSelection(_ id: E.ID) {
        self.editCoordinator.requestTransition { self.editorState.select(id) }
    }

    private func registerEditSession(isDirty: Bool) {
        self.editCoordinator.register(
            isDirty: isDirty,
            save: { await self.saveEntity() },
            discard: self.discardChanges
        )
    }

    private func discardChanges() {
        if let entity = self.selectedEntity {
            self.editorContent = entity.content
            self.editorState.setDirty(false)
        }
    }
}
