import SwiftUI

struct WorkView: View {
    @Bindable var model: PhoneModel
    @State private var showsNewGoal = false
    @State private var openJob: AgentJobWire?
    @Environment(\.theme) private var theme

    private var currentGoals: [Goal] {
        self.model.work.goals
            .filter { $0.status == .active || $0.status == .paused }
            .sorted { ($0.status == .active ? 0 : 1, -$0.priority) < ($1.status == .active ? 0 : 1, -$1.priority) }
    }

    private var finishedGoals: [Goal] {
        self.model.work.goals.filter { $0.status == .completed }
    }

    private var goalsNote: String {
        if self.model.work.goalsLoad.loaded {
            return L10n.tr("phone.work.goals.empty")
        }
        if self.model.work.goalsLoad.failed {
            return L10n.tr("phone.work.goals.failed")
        }
        return L10n.tr("phone.work.loading")
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 24) {
                    if self.model.needsAttention {
                        ConnectionCard(model: self.model)
                    } else if self.model.workFailed {
                        PhoneTroubleNote(text: L10n.tr("phone.load.stale"))
                    }
                    if !self.model.questions.isEmpty {
                        WorkSection(title: L10n.tr("phone.work.needs_you")) {
                            ForEach(self.model.questions) { question in
                                QuestionCard(model: self.model, question: question)
                            }
                        }
                    }
                    WorkSection(title: L10n.tr("phone.work.goals")) {
                        if self.currentGoals.isEmpty {
                            WorkEmptyNote(text: self.goalsNote)
                        }
                        ForEach(self.currentGoals) { goal in
                            NavigationLink(value: goal.id) {
                                GoalRow(goal: goal)
                            }
                            .buttonStyle(BobePressStyle(scale: 0.985))
                        }
                        Button {
                            self.showsNewGoal = true
                        } label: {
                            Label(L10n.tr("phone.work.goals.new"), systemImage: "plus")
                        }
                        .buttonStyle(.phoneQuiet)
                        .disabled(self.model.link != .online)
                    }
                    if !self.model.work.schedules.isEmpty {
                        WorkSection(title: L10n.tr("phone.work.coming_up")) {
                            ForEach(self.model.work.schedules.filter(\.enabled)) { schedule in
                                ScheduleRow(schedule: schedule)
                            }
                        }
                    }
                    if !self.model.work.jobs.isEmpty {
                        WorkSection(title: L10n.tr("phone.work.recently")) {
                            ForEach(self.model.work.jobs.prefix(8)) { job in
                                Button {
                                    self.openJob = job
                                } label: {
                                    JobRow(job: job)
                                }
                                .buttonStyle(BobePressStyle(scale: 0.985))
                            }
                        }
                    }
                    if !self.finishedGoals.isEmpty {
                        WorkSection(title: L10n.tr("phone.work.done")) {
                            ForEach(self.finishedGoals) { goal in
                                NavigationLink(value: goal.id) {
                                    GoalRow(goal: goal)
                                }
                                .buttonStyle(BobePressStyle(scale: 0.985))
                            }
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 8)
                .padding(.bottom, 24)
                .animation(BobeMotion.standard, value: self.model.work.goals.map(\.id))
            }
            .background(PhoneBackdrop())
            .navigationTitle(L10n.tr("phone.work.title"))
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    SettingsButton(model: self.model)
                }
            }
            .navigationDestination(for: String.self) { id in
                GoalDetailView(model: self.model, goalId: id)
            }
            .refreshable { await self.model.loadWork() }
            .task(id: self.model.isConnected) {
                if self.model.isConnected {
                    await self.model.loadWork()
                }
            }
            .sheet(isPresented: self.$showsNewGoal) {
                NewGoalSheet(model: self.model)
                    .presentationDetents([.medium, .large])
            }
            .sheet(item: self.$openJob) { job in
                JobSheet(job: job)
                    .presentationDetents([.medium, .large])
                    .presentationDragIndicator(.visible)
            }
        }
    }
}

private struct WorkSection<Content: View>: View {
    let title: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            PhoneSectionLabel(text: self.title)
                .padding(.horizontal, 4)
            self.content
        }
    }
}

private struct WorkEmptyNote: View {
    let text: String
    @Environment(\.theme) private var theme

    var body: some View {
        Text(self.text)
            .phoneText(.callout)
            .foregroundStyle(self.theme.colors.textMuted)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(16)
            .bobeWell(cornerRadius: BobeRadius.card)
    }
}

struct GoalStatusPill: View {
    let status: GoalStatus
    @Environment(\.theme) private var theme

    private var tint: Color {
        switch self.status {
        case .active: self.theme.colors.success
        case .completed: self.theme.colors.secondary
        case .paused, .archived, .unknown: self.theme.colors.textMuted
        }
    }

    var body: some View {
        Text(L10n.tr("phone.goal.status.\(self.status.rawValue)"))
            .phoneText(.label)
            .foregroundStyle(self.tint)
            .padding(.horizontal, 9)
            .padding(.vertical, 4)
            .background(self.tint.opacity(0.12), in: Capsule(style: .continuous))
    }
}

private struct GoalRow: View {
    let goal: Goal
    @Environment(\.theme) private var theme

    var body: some View {
        PhoneCard {
            HStack(alignment: .top, spacing: 12) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(self.goal.title)
                            .phoneText(.heading)
                            .foregroundStyle(self.theme.colors.text)
                            .multilineTextAlignment(.leading)
                        Spacer(minLength: 4)
                        GoalStatusPill(status: self.goal.status)
                    }
                    if !self.goal.summary.isEmpty {
                        Text(self.goal.summary)
                            .phoneText(.callout)
                            .foregroundStyle(self.theme.colors.textMuted)
                            .lineLimit(2)
                            .multilineTextAlignment(.leading)
                    }
                    if let updated = PhoneDates.relative(self.goal.updatedAt) {
                        Text(L10n.tr("phone.goal.updated_format", updated))
                            .phoneText(.footnote)
                            .foregroundStyle(self.theme.colors.textMuted)
                    }
                }
                Image(systemName: "chevron.right")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(self.theme.colors.textMuted)
                    .padding(.top, 4)
            }
        }
        .opacity(self.goal.status == .paused ? 0.8 : 1)
    }
}

private struct JobRow: View {
    let job: AgentJobWire
    @Environment(\.theme) private var theme

    var body: some View {
        PhoneCard(padding: 14) {
            HStack(alignment: .top, spacing: 12) {
                self.icon
                    .frame(width: 24, height: 24)
                VStack(alignment: .leading, spacing: 4) {
                    Text(self.job.title)
                        .phoneText(.heading)
                        .foregroundStyle(self.theme.colors.text)
                        .multilineTextAlignment(.leading)
                    Text(self.detail)
                        .phoneText(.callout)
                        .foregroundStyle(self.theme.colors.textMuted)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                }
                Spacer(minLength: 0)
            }
        }
    }

    private var detail: String {
        switch self.job.status {
        case .completed:
            self.job.output.map(PhoneDates.firstLine) ?? L10n.tr("phone.job.status.completed")
        case .failed, .interrupted:
            L10n.tr("phone.job.status.failed")
        case .running, .cancelRequested:
            L10n.tr("phone.job.status.running")
        case .queued:
            L10n.tr("phone.job.status.queued")
        case .cancelled:
            L10n.tr("phone.job.status.cancelled")
        case .awaitingInput:
            L10n.tr("phone.job.status.waiting")
        }
    }

    @ViewBuilder
    private var icon: some View {
        switch self.job.status {
        case .running, .cancelRequested:
            ProgressView().controlSize(.small)
        case .queued:
            Image(systemName: "clock").foregroundStyle(self.theme.colors.textMuted)
        case .completed:
            Image(systemName: "checkmark.circle.fill").foregroundStyle(self.theme.colors.success)
        case .failed, .interrupted:
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(self.theme.colors.warning)
        case .cancelled:
            Image(systemName: "xmark.circle").foregroundStyle(self.theme.colors.textMuted)
        case .awaitingInput:
            Image(systemName: "questionmark.circle.fill").foregroundStyle(self.theme.colors.primary)
        }
    }
}

private struct ScheduleRow: View {
    let schedule: AgentScheduleWire
    @Environment(\.theme) private var theme

    private var repeats: String? {
        switch self.schedule.everySeconds {
        case nil: nil
        case 3600: L10n.tr("phone.schedule.hourly")
        case 86400: L10n.tr("phone.schedule.daily")
        case 604_800: L10n.tr("phone.schedule.weekly")
        default: L10n.tr("phone.schedule.repeats")
        }
    }

    var body: some View {
        PhoneCard(padding: 14) {
            HStack(alignment: .top, spacing: 12) {
                Image(systemName: "calendar.badge.clock")
                    .foregroundStyle(self.theme.colors.primary)
                    .frame(width: 24, height: 24)
                VStack(alignment: .leading, spacing: 4) {
                    Text(self.schedule.title)
                        .phoneText(.heading)
                        .foregroundStyle(self.theme.colors.text)
                    HStack(spacing: 6) {
                        if let next = self.schedule.nextRunDate {
                            Text(next, format: .relative(presentation: .named))
                        }
                        if let repeats {
                            Text(verbatim: "·")
                            Text(repeats)
                        }
                    }
                    .phoneText(.callout)
                    .foregroundStyle(self.theme.colors.textMuted)
                }
                Spacer(minLength: 0)
            }
        }
    }
}

private struct JobSheet: View {
    let job: AgentJobWire
    @Environment(\.theme) private var theme

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text(self.job.title)
                    .phoneText(.title)
                    .foregroundStyle(self.theme.colors.text)
                    .padding(.top, 24)
                Text(self.job.output ?? self.job.error ?? self.job.task)
                    .phoneText(.body)
                    .foregroundStyle(self.theme.colors.text)
                    .textSelection(.enabled)
            }
            .padding(.horizontal, 24)
            .padding(.bottom, 24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(PhoneBackdrop())
        .phoneThemed()
    }
}

enum PhoneDates {
    static func parse(_ value: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return fractional.date(from: value) ?? ISO8601DateFormatter().date(from: value)
    }

    static func relative(_ value: String) -> String? {
        guard let date = self.parse(value) else { return nil }
        return date.formatted(.relative(presentation: .named))
    }

    static func firstLine(_ text: String) -> String {
        text.split(whereSeparator: \.isNewline).first.map(String.init) ?? text
    }
}
