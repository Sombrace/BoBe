import Foundation

extension GoalsEditor {
    func loadGoals() async {
        let selection = EndpointStore.shared.selection
        let client = DaemonClient(endpoint: selection.endpoint)
        self.editorState.setLoading(true)
        defer { self.editorState.setLoading(false) }
        do {
            let resp = try await client.listGoals(
                status: self.statusFilter,
                includeArchived: self.showArchived || self.statusFilter == .archived
            )
            guard EndpointStore.shared.selection == selection else { return }
            let previous = self.selectedGoal
            self.goals = resp.goals
            if let previous, let updated = self.selectedGoal {
                self.draft = self.draft.reconciled(submitted: GoalDraft(previous), saved: GoalDraft(updated))
                self.editorState.setDirty(!self.draft.equals(updated))
            }
            if let selected = self.editorState.selectedId, !self.goals.contains(where: { $0.id == selected }) {
                self.editorState.select(self.goals.first?.id)
            } else if self.editorState.selectedId == nil {
                self.editorState.select(self.goals.first?.id)
            }
            self.editorState.clearError()
        } catch {
            self.editorState.setError(error)
        }
    }

    func createGoal() {
        let title = self.newTitle.trimmingCharacters(in: .whitespaces)
        guard !title.isEmpty else { return }
        Task {
            do {
                var req = GoalCreateRequest(title: title)
                req.priority = self.newPriority
                let goal = try await DaemonClient.shared.createGoal(req)
                self.goals.insert(goal, at: 0)
                self.newTitle = ""
                self.newPriority = 2
                self.editCoordinator.selectCreated(goal.id, in: self.$editorState)
            } catch {
                self.editorState.setError(error)
            }
        }
    }

    func saveGoal() async -> Bool {
        guard let id = self.editorState.selectedId, !self.editorState.isSaving else { return false }
        let submitted = self.draft
        let selection = EndpointStore.shared.selection
        let client = DaemonClient(endpoint: selection.endpoint)
        self.editorState.setSaving(true)
        defer { self.editorState.setSaving(false) }
        do {
            var req = GoalUpdateRequest()
            req.title = submitted.title
            req.status = submitted.status
            req.priority = submitted.priority
            req.summary = submitted.summary
            req.whyItMatters = submitted.whyItMatters
            req.notes = submitted.notes
            let updated = try await client.updateGoal(id, req)
            guard EndpointStore.shared.selection == selection else { return false }
            if let idx = self.goals.firstIndex(where: { $0.id == id }) {
                self.goals[idx] = updated
            }
            guard self.editorState.selectedId == id else { return false }
            self.draft = self.draft.reconciled(submitted: submitted, saved: GoalDraft(updated))
            self.editorState.setDirty(!self.draft.equals(updated))
            self.settleFilteredSelection()
            return !self.editorState.isDirty
        } catch {
            self.editorState.setError(error)
            return false
        }
    }

    func deleteGoal(_ goal: Goal) {
        Task {
            do {
                try await DaemonClient.shared.deleteGoal(goal.id)
                self.goals.removeAll { $0.id == goal.id }
                if self.editorState.selectedId == goal.id {
                    self.editorState.select(self.goals.first?.id)
                }
                self.editorState.dismissDeleteConfirmation()
            } catch {
                self.editorState.setError(error)
            }
        }
    }

    func changeStatus(_ goal: Goal, to status: GoalStatus) {
        guard !self.editorState.isSaving else { return }
        let selection = EndpointStore.shared.selection
        let client = DaemonClient(endpoint: selection.endpoint)
        let submittedStatus = self.draft.status
        self.editorState.setSaving(true)
        Task {
            defer { self.editorState.setSaving(false) }
            do {
                var req = GoalUpdateRequest()
                req.status = status
                let updated = try await client.updateGoal(goal.id, req)
                guard EndpointStore.shared.selection == selection else { return }
                if let idx = self.goals.firstIndex(where: { $0.id == goal.id }) {
                    self.goals[idx] = updated
                }
                if self.editorState.selectedId == goal.id {
                    let latestStatus = self.draft.status
                    self.draft = self.draft.reconciled(
                        submitted: GoalDraft(goal), saved: GoalDraft(updated)
                    )
                    self.draft.status = reconcileSavedDraft(
                        current: latestStatus, submitted: submittedStatus, saved: updated.status
                    )
                    self.editorState.setDirty(!self.draft.equals(updated))
                    self.settleFilteredSelection()
                }
            } catch {
                self.editorState.setError(error)
            }
        }
    }

    func settleFilteredSelection() {
        guard !self.editorState.isDirty,
              let goal = self.selectedGoal,
              !self.goalMatchesFilter(goal)
        else { return }
        self.editorState.select(self.visibleGoals.first?.id)
    }

    func completeGoal(_ goal: Goal) {
        self.changeStatus(goal, to: .completed)
    }

    func archiveGoal(_ goal: Goal) {
        self.changeStatus(goal, to: .archived)
    }
}
