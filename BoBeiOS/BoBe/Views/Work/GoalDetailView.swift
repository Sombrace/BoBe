import SwiftUI

struct GoalDetailView: View {
    let model: PhoneModel
    let goalId: String
    @State private var confirmsDone = false
    @Environment(\.theme) private var theme

    private var goal: Goal? {
        self.model.goals.first { $0.id == self.goalId }
    }

    var body: some View {
        ScrollView {
            if let goal {
                VStack(alignment: .leading, spacing: 20) {
                    VStack(alignment: .leading, spacing: 10) {
                        Text(goal.title)
                            .phoneText(.title)
                            .foregroundStyle(self.theme.colors.text)
                        HStack(spacing: 8) {
                            GoalStatusPill(status: goal.status)
                            if let updated = PhoneDates.relative(goal.updatedAt) {
                                Text(L10n.tr("phone.goal.updated_format", updated))
                                    .phoneText(.footnote)
                                    .foregroundStyle(self.theme.colors.textMuted)
                            }
                        }
                    }
                    ForEach(self.sections(of: goal), id: \.0) { section in
                        PhoneCard {
                            VStack(alignment: .leading, spacing: 8) {
                                PhoneSectionLabel(text: section.0)
                                Text(Self.rendered(section.1))
                                    .phoneText(.body)
                                    .foregroundStyle(self.theme.colors.text)
                                    .textSelection(.enabled)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                    if self.sections(of: goal).isEmpty {
                        Text(L10n.tr("phone.goal.empty"))
                            .phoneText(.callout)
                            .foregroundStyle(self.theme.colors.textMuted)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .background(PhoneBackdrop())
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            if let goal, goal.status == .active || goal.status == .paused {
                ToolbarItem(placement: .topBarTrailing) {
                    Menu {
                        Button {
                            Task { await self.model.setGoal(goal, paused: goal.status == .active) }
                        } label: {
                            Label(
                                L10n.tr(goal.status == .active ? "phone.goal.pause" : "phone.goal.resume"),
                                systemImage: goal.status == .active ? "pause.circle" : "play.circle"
                            )
                        }
                        Button {
                            self.confirmsDone = true
                        } label: {
                            Label(L10n.tr("phone.goal.complete"), systemImage: "checkmark.circle")
                        }
                    } label: {
                        Image(systemName: "ellipsis.circle")
                    }
                    .accessibilityLabel(L10n.tr("phone.goal.actions"))
                    .disabled(self.model.link != .online)
                }
            }
        }
        .confirmationDialog(
            L10n.tr("phone.goal.complete.title"),
            isPresented: self.$confirmsDone,
            titleVisibility: .visible
        ) {
            Button(L10n.tr("phone.goal.complete")) {
                if let goal {
                    Task { await self.model.completeGoal(goal) }
                }
            }
        }
    }

    private func sections(of goal: Goal) -> [(String, String)] {
        [
            (L10n.tr("phone.goal.section.summary"), goal.summary),
            (L10n.tr("phone.goal.section.why"), goal.whyItMatters),
            (L10n.tr("phone.goal.section.how"), goal.howWorkingOnIt),
            (L10n.tr("phone.goal.section.noticed"), goal.patternsObserved),
            (L10n.tr("phone.goal.section.questions"), goal.openQuestions),
            (L10n.tr("phone.goal.section.notes"), goal.notes),
        ].filter { !$0.1.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    private static func rendered(_ text: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }
}

struct NewGoalSheet: View {
    let model: PhoneModel
    @State private var title = ""
    @State private var why = ""
    @State private var saving = false
    @State private var failed = false
    @FocusState private var focused: Bool
    @Environment(\.dismiss) private var dismiss
    @Environment(\.theme) private var theme

    private var canSave: Bool {
        !self.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !self.saving
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(L10n.tr("phone.goal.new.title"))
                .phoneText(.title)
                .foregroundStyle(self.theme.colors.text)
                .padding(.top, 28)
            TextField(L10n.tr("phone.goal.new.name"), text: self.$title)
                .focused(self.$focused)
                .padding(.horizontal, 16)
                .frame(minHeight: 52)
                .bobeWell(cornerRadius: BobeRadius.field)
            TextField(L10n.tr("phone.goal.new.why"), text: self.$why, axis: .vertical)
                .lineLimit(2 ... 5)
                .padding(.horizontal, 16)
                .padding(.vertical, 14)
                .bobeWell(cornerRadius: BobeRadius.field)
            if self.failed {
                Text(L10n.tr("phone.goal.new.failed"))
                    .phoneText(.footnote)
                    .foregroundStyle(self.theme.colors.warning)
            }
            Spacer(minLength: 0)
            Button(L10n.tr("phone.goal.new.save")) {
                self.saving = true
                Task {
                    let saved = await self.model.createGoal(title: self.title, why: self.why)
                    self.saving = false
                    if saved {
                        self.dismiss()
                    } else {
                        self.failed = true
                    }
                }
            }
            .buttonStyle(.phonePrimary)
            .disabled(!self.canSave)
        }
        .padding(.horizontal, 24)
        .padding(.bottom, 12)
        .background(PhoneBackdrop())
        .phoneThemed()
        .onAppear { self.focused = true }
    }
}
