import SwiftUI

extension GoalsEditor {
    @ViewBuilder
    var detailPane: some View {
        if let goal = self.selectedGoal {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    self.detailHeader(goal)
                    self.statusActionRow(goal)

                    self.labeledField(L10n.tr("settings.goals.section.title")) {
                        BobeTextField(
                            placeholder: L10n.tr("settings.goals.new.placeholder"),
                            text: self.$draft.title
                        )
                    }

                    HStack(alignment: .top, spacing: 16) {
                        self.labeledField(L10n.tr("settings.goals.section.status")) {
                            BobeMenuPicker(
                                selection: self.$draft.status,
                                options: Self.statusEditOptions,
                                label: { Self.statusLabel($0) },
                                width: 200
                            )
                        }
                        self.labeledField(
                            L10n.tr("settings.goals.section.priority"),
                            hint: L10n.tr("settings.goals.priority.range_hint")
                        ) {
                            BobeMenuPicker(
                                selection: self.$draft.priority,
                                options: Self.priorityOptions,
                                label: { Self.priorityLabel($0) },
                                width: 140
                            )
                        }
                        Spacer()
                    }

                    self.labeledField(L10n.tr("settings.goals.section.summary")) {
                        BobeProseEditor(
                            text: self.$draft.summary,
                            accessibilityLabel: L10n.tr("settings.goals.section.summary"),
                            minHeight: 80
                        )
                    }
                    self.labeledField(L10n.tr("settings.goals.section.why_it_matters")) {
                        BobeProseEditor(
                            text: self.$draft.whyItMatters,
                            accessibilityLabel: L10n.tr("settings.goals.section.why_it_matters"),
                            minHeight: 100
                        )
                    }
                    self.labeledField(L10n.tr("settings.goals.section.notes")) {
                        BobeProseEditor(
                            text: self.$draft.notes,
                            accessibilityLabel: L10n.tr("settings.goals.section.notes"),
                            minHeight: 220
                        )
                    }

                    VStack(alignment: .leading, spacing: 8) {
                        self.aiCuratedSection(
                            title: L10n.tr("settings.goals.section.how_working_on_it"),
                            icon: "hammer.fill",
                            body: goal.howWorkingOnIt
                        )
                        self.aiCuratedSection(
                            title: L10n.tr("settings.goals.section.patterns_observed"),
                            icon: "waveform.path.ecg",
                            body: goal.patternsObserved
                        )
                        self.aiCuratedSection(
                            title: L10n.tr("settings.goals.section.attitude_feelings"),
                            icon: "heart.fill",
                            body: goal.attitudeFeelings
                        )
                        self.aiCuratedSection(
                            title: L10n.tr("settings.goals.section.open_questions"),
                            icon: "questionmark.bubble.fill",
                            body: goal.openQuestions
                        )
                    }

                    SettingsEditorActionRow {
                        self.deleteRow(goal)
                    } trailing: {
                        SettingsEditorSaveActions(
                            isDirty: self.editorState.isDirty,
                            isSaving: self.editorState.isSaving,
                            onDiscard: { self.draft = GoalDraft(goal) },
                            onSave: { Task { _ = await self.saveGoal() } }
                        )
                    }

                    if let errorMessage = self.editorState.errorMessage {
                        SettingsEditorErrorText(message: errorMessage)
                    }
                }
                .padding(.bottom, 24)
            }
        }
    }

    func labeledField(
        _ label: String,
        hint: String? = nil,
        @ViewBuilder content: () -> some View
    ) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label)
                .bobeTextStyle(.rowMeta)
                .fontWeight(.semibold)
                .foregroundStyle(self.theme.colors.text)
            content()
            if let hint {
                Text(hint)
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
            }
        }
    }

    func detailHeader(_ goal: Goal) -> some View {
        HStack(spacing: 6) {
            Text(L10n.tr("settings.goals.badge.goal"))
                .font(.system(size: 11, weight: .medium))
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Capsule().fill(self.theme.colors.border.opacity(0.4)))

            if self.editorState.isDirty {
                Text(L10n.tr("settings.editor.badge.unsaved"))
                    .font(.system(size: 9, weight: .medium))
                    .foregroundStyle(self.theme.colors.tertiary)
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(self.theme.colors.tertiary.opacity(0.15)))
            }

            Text(Self.priorityLabel(self.draft.priority))
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(self.priorityColor(self.draft.priority))
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Capsule().fill(self.priorityColor(self.draft.priority).opacity(0.15)))

            Text(Self.statusLabel(self.draft.status).uppercased())
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(self.statusColor(self.draft.status))
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .background(Capsule().fill(self.statusColor(self.draft.status).opacity(0.15)))

            Spacer()

            Text(Self.relativeDate(goal.updatedAt))
                .bobeTextStyle(.badge)
                .foregroundStyle(self.theme.colors.textMuted)
        }
    }

    func statusActionRow(_ goal: Goal) -> some View {
        HStack(spacing: 6) {
            Spacer()
            switch goal.status {
            case .active:
                self.statusButton(L10n.tr("settings.goals.action.pause"), icon: "pause.circle") {
                    self.changeStatus(goal, to: .paused)
                }
                self.statusButton(L10n.tr("settings.goals.action.complete"), icon: "checkmark.circle") {
                    self.completeGoal(goal)
                }
                self.statusButton(L10n.tr("settings.goals.action.archive"), icon: "archivebox") {
                    self.archiveGoal(goal)
                }
            case .paused:
                self.statusButton(L10n.tr("settings.goals.action.resume"), icon: "play.circle") {
                    self.changeStatus(goal, to: .active)
                }
                self.statusButton(L10n.tr("settings.goals.action.complete"), icon: "checkmark.circle") {
                    self.completeGoal(goal)
                }
                self.statusButton(L10n.tr("settings.goals.action.archive"), icon: "archivebox") {
                    self.archiveGoal(goal)
                }
            case .completed:
                self.statusButton(L10n.tr("settings.goals.action.reactivate"), icon: "arrow.uturn.backward.circle") {
                    self.changeStatus(goal, to: .active)
                }
            case .archived:
                self.statusButton(L10n.tr("settings.goals.action.restore"), icon: "tray.and.arrow.up") {
                    self.changeStatus(goal, to: .active)
                }
            case .unknown:
                EmptyView()
            }
        }
    }

    func statusButton(_ title: String, icon: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 4) {
                Image(systemName: icon)
                Text(title)
            }
        }
        .bobeButton(.secondary, size: .small)
        .disabled(self.editorState.isSaving)
    }

    static func priorityLabel(_ priority: Int) -> String {
        L10n.tr("settings.goals.priority.\(min(max(priority, 0), 5))")
    }

    func aiCuratedSection(title: String, icon: String, body: String) -> some View {
        CollapsibleSection(
            title: title,
            icon: icon,
            aiCuratedBadge: true,
            initiallyExpanded: false
        ) {
            if body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                Text(L10n.tr("settings.goals.section.ai_empty"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted.opacity(0.8))
                    .italic()
            } else {
                Text(body)
                    .bobeTextStyle(.settingsBody)
                    .foregroundStyle(self.theme.colors.text)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .textSelection(.enabled)
            }
        }
    }

    func deleteRow(_ goal: Goal) -> some View {
        Group {
            if self.editorState.showDeleteConfirmation {
                HStack(spacing: 6) {
                    Text(L10n.tr("settings.editor.delete.confirm"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.primary)
                    Button(L10n.tr("settings.editor.delete.yes")) {
                        self.deleteGoal(goal)
                    }
                    .bobeButton(.destructive, size: .small)
                    Button(L10n.tr("settings.editor.delete.no")) {
                        self.editorState.dismissDeleteConfirmation()
                    }
                    .bobeButton(.secondary, size: .small)
                }
            } else {
                Button {
                    self.editorState.requestDeleteConfirmation()
                } label: {
                    Image(systemName: "trash")
                }
                .accessibilityLabel(L10n.tr("settings.goals.delete.accessibility"))
                .bobeButton(.destructive, size: .small)
            }
        }
    }
}
