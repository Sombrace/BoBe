import SwiftUI

struct AgentSchedulesSection: View {
    let selection: EndpointSelection
    @Environment(\.theme) private var theme
    @State private var schedules: [AgentScheduleWire]?
    @State private var error: String?
    @State private var working = false
    @State private var title = ""
    @State private var task = ""
    @State private var at = Date.now.addingTimeInterval(3600)
    @State private var repeats = false
    @State private var minutes = 60
    @State private var requestId = UUID()
    @State private var previousCreation: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L10n.tr("settings.agent.schedules.title")).bobeTextStyle(.setupHeading)
                .help(L10n.tr("settings.agent.schedules.description"))
            if let schedules {
                if schedules.isEmpty {
                    Text(L10n.tr("settings.agent.schedules.empty"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                }
                ForEach(schedules) { schedule in
                    self.scheduleCard(schedule)
                }
            } else {
                BobeSpinner(size: 12)
            }
            BobeDisclosure(L10n.tr("settings.agent.schedules.create")) {
                VStack(alignment: .leading, spacing: 8) {
                    TextField(L10n.tr("settings.agent.schedules.name"), text: self.$title)
                        .bobeField()
                    TextField(L10n.tr("settings.agent.jobs.task"), text: self.$task, axis: .vertical)
                        .lineLimit(2 ... 5)
                        .bobeField()
                    DatePicker(L10n.tr("settings.agent.schedules.time"), selection: self.$at)
                    Toggle(L10n.tr("settings.agent.schedules.repeat"), isOn: self.$repeats)
                        .toggleStyle(BobeLabeledSwitchStyle())
                    if self.repeats {
                        Stepper(L10n.tr("settings.agent.schedules.interval_minutes", self.minutes),
                                value: self.$minutes, in: 1 ... 525600)
                    }
                    Button(L10n.tr("settings.agent.schedules.create")) { Task { await self.create() } }
                        .bobeButton(.primary, size: .small)
                        .disabled(self.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                            || self.task.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
                .padding(.top, 4)
            }
            if let error { SettingsErrorBanner(message: error) }
        }
        .disabled(self.working)
        .task(id: self.selection) {
            self.schedules = nil
            self.error = nil
            while !Task.isCancelled {
                if !self.working { await self.refresh() }
                do { try await Task.sleep(for: .seconds(5)) } catch { return }
            }
        }
    }

    private func scheduleCard(_ schedule: AgentScheduleWire) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 12) {
                BobeIconTile(
                    systemName: schedule.everySeconds == nil ? "clock" : "repeat",
                    tint: schedule.enabled ? self.theme.colors.primary : self.theme.colors.textMuted,
                    size: 32
                )
                VStack(alignment: .leading, spacing: 2) {
                    Text(schedule.title)
                        .bobeTextStyle(.rowTitle)
                        .foregroundStyle(self.theme.colors.text)
                        .lineLimit(2)
                    HStack(spacing: 6) {
                        if let date = schedule.nextRunDate {
                            Text(date, format: .dateTime.month().day().hour().minute())
                        }
                        if let seconds = schedule.everySeconds {
                            Text(L10n.tr("settings.agent.schedules.interval_seconds", seconds))
                        }
                    }
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                }
                Spacer(minLength: 8)
                BobeToggle(
                    isOn: Binding(
                        get: { schedule.enabled },
                        set: { enabled in Task { await self.setEnabled(schedule, enabled: enabled) } }
                    ),
                    accessibilityLabel: schedule.title
                )
            }
            if let error = schedule.lastError {
                Text(error)
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.error)
                    .fixedSize(horizontal: false, vertical: true)
            }
            BobeDisclosure(L10n.tr("settings.agent.jobs.task")) {
                VStack(alignment: .leading, spacing: 10) {
                    Text(schedule.task)
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.text)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                    Button(L10n.tr("settings.agent.schedules.cancel")) {
                        Task { await self.cancel(schedule.id) }
                    }
                    .bobeButton(.ghost, size: .small)
                }
            }
        }
        .padding(14)
        .bobeRaised(cornerRadius: BobeRadius.card)
        .opacity(schedule.enabled ? 1 : 0.72)
        .animation(BobeMotion.standard, value: schedule.enabled)
    }

    private func refresh() async {
        do {
            let schedules = try await DaemonClient(endpoint: self.selection.endpoint).agentSchedules()
            guard EndpointStore.shared.selection == self.selection else { return }
            self.schedules = schedules
            self.error = nil
        } catch {
            guard !Task.isCancelled, EndpointStore.shared.selection == self.selection else { return }
            self.error = error.localizedDescription
        }
    }

    private func change(_ operation: () async throws -> Void) async {
        guard !self.working, EndpointStore.shared.selection == self.selection else { return }
        self.working = true
        defer { self.working = false }
        do {
            try await operation()
            guard EndpointStore.shared.selection == self.selection else { return }
            await self.refresh()
        } catch {
            guard EndpointStore.shared.selection == self.selection else { return }
            self.error = error.localizedDescription
        }
    }

    private func setEnabled(_ schedule: AgentScheduleWire, enabled: Bool) async {
        await self.change {
            _ = try await DaemonClient(endpoint: self.selection.endpoint).setScheduleEnabled(schedule.id, enabled: enabled)
        }
    }

    private func cancel(_ id: UUID) async {
        await self.change { try await DaemonClient(endpoint: self.selection.endpoint).cancelSchedule(id) }
    }

    private func create() async {
        let signature = "\(self.title):\(self.task):\(self.at.timeIntervalSince1970):\(self.repeats):\(self.minutes)"
        if self.previousCreation != signature {
            self.previousCreation = signature
            self.requestId = UUID()
        }
        await self.change {
            _ = try await DaemonClient(endpoint: self.selection.endpoint).createSchedule(
                title: self.title, task: self.task, at: self.at,
                everySeconds: self.repeats ? self.minutes * 60 : nil, requestId: self.requestId
            )
            guard EndpointStore.shared.selection == self.selection else { return }
            self.title = ""
            self.task = ""
        }
    }
}
