import SwiftUI

struct AgentWorkSection: View {
    var showsTools = true
    var showsWork = true
    @Environment(\.theme) private var theme
    @State private var selection: AgentToolSelection?
    @State private var jobs: [AgentJobWire] = []
    @State private var endpoint = EndpointStore.shared.selection
    @State private var error: String?
    @State private var working = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if self.showsTools {
                if let selection {
                    ForEach(selection.tools) { tool in
                        Toggle(isOn: Binding(
                            get: { tool.enabled },
                            set: { enabled in Task { await self.setTool(tool.name, enabled: enabled) } }
                        )) {
                            HStack {
                                Text(self.toolLabel(tool.name))
                                    .bobeTextStyle(.settingsBody)
                                if tool.active != tool.enabled {
                                    Text(L10n.tr("settings.agent.tools.pending"))
                                        .bobeTextStyle(.helper)
                                        .foregroundStyle(self.theme.colors.warning)
                                }
                            }
                            .help(self.toolDescription(tool.name))
                        }
                        .toggleStyle(BobeLabeledSwitchStyle())
                    }
                    if selection.restartRequired {
                        Text(L10n.tr("settings.providers.pending"))
                            .bobeTextStyle(.helper)
                            .help(L10n.tr("settings.agent.tools.restart"))
                        if self.endpoint.endpoint.canRestartRuntime {
                            Button(L10n.tr("settings.shared.restart_banner.restart_now")) {
                                Task { await self.restart() }
                            }
                            .bobeButton(.secondary, size: .small)
                        } else {
                            Label(L10n.tr("settings.providers.host"), systemImage: "server.rack")
                                .bobeTextStyle(.helper)
                                .help(L10n.tr("settings.providers.restart.external"))
                        }
                    }
                }
                Label(L10n.tr("settings.providers.capability_note"), systemImage: "info.circle")
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .help(L10n.tr("settings.agent.tools.description"))
            }
            if self.showsTools && self.showsWork { Divider() }
            if self.showsWork {
                Text(L10n.tr("settings.agent.jobs.title")).bobeTextStyle(.setupHeading)
                    .help(L10n.tr("settings.agent.jobs.description"))
                if self.selection != nil && self.jobs.isEmpty {
                    Text(L10n.tr("settings.agent.jobs.empty"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                }
                ForEach(self.jobs) { job in
                    VStack(alignment: .leading, spacing: 6) {
                        HStack(alignment: .firstTextBaseline) {
                            Text(job.title).bobeTextStyle(.settingsBody).fontWeight(.medium)
                                .textSelection(.enabled)
                            Spacer()
                            self.jobStatus(job.status)
                        }
                        if let error = job.error {
                            Text(error).bobeTextStyle(.helper).foregroundStyle(self.theme.colors.error)
                        }
                        BobeDisclosure(L10n.tr("settings.agent.jobs.task")) {
                            Text(job.task).bobeTextStyle(.helper).textSelection(.enabled)
                        }
                        if let output = job.output, !output.isEmpty {
                            BobeDisclosure(L10n.tr("settings.agent.jobs.result")) {
                                Text(output).bobeTextStyle(.helper).textSelection(.enabled)
                            }
                        }
                        HStack {
                            if job.canCancel {
                                Button(L10n.tr("settings.agent.jobs.cancel")) {
                                    Task { await self.changeJob(job, resume: false) }
                                }.bobeButton(.secondary, size: .small)
                            }
                            if job.canResume {
                                Button(L10n.tr("settings.agent.jobs.resume")) {
                                    Task { await self.changeJob(job, resume: true) }
                                }.bobeButton(.secondary, size: .small)
                            }
                        }
                    }
                    .padding(.vertical, 6)
                }
                BobeHairline()
                AgentSchedulesSection(selection: self.endpoint)
            }
            if self.working || (self.selection == nil && self.error == nil) {
                BobeSpinner(size: 12)
            }
            if let error { SettingsErrorBanner(message: error) }
        }
        .disabled(self.working)
        .task(id: self.endpoint) {
            while !Task.isCancelled {
                if !self.working { await self.refresh() }
                do {
                    try await Task.sleep(for: .seconds(5))
                } catch {
                    return
                }
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: .bobeEndpointIdentityChanged)) { _ in
            self.endpoint = EndpointStore.shared.selection
            self.selection = nil
            self.jobs = []
            self.error = nil
        }
    }

    private func refresh() async {
        let endpoint = EndpointStore.shared.selection
        do {
            let client = DaemonClient(endpoint: endpoint.endpoint)
            async let tools = client.agentTools()
            async let jobs = client.backgroundJobs()
            let result = try await (tools, jobs)
            guard EndpointStore.shared.selection == endpoint else { return }
            self.endpoint = endpoint
            self.selection = result.0
            self.jobs = result.1
            self.error = nil
        } catch {
            guard EndpointStore.shared.selection == endpoint else { return }
            self.error = error.localizedDescription
        }
    }

    private func setTool(_ tool: AgentToolWire, enabled: Bool) async {
        guard !self.working, let selection, EndpointStore.shared.selection == self.endpoint else { return }
        let endpoint = self.endpoint
        var tools = Set(selection.tools.filter(\.enabled).map(\.name))
        if enabled { tools.insert(tool) } else { tools.remove(tool) }
        self.working = true
        defer { self.working = false }
        do {
            let result = try await DaemonClient(endpoint: endpoint.endpoint).setAgentTools(Array(tools))
            guard EndpointStore.shared.selection == endpoint else { return }
            self.selection = result
            self.error = nil
        } catch {
            guard EndpointStore.shared.selection == endpoint else { return }
            self.error = error.localizedDescription
        }
    }

    private func changeJob(_ job: AgentJobWire, resume: Bool) async {
        guard !self.working, EndpointStore.shared.selection == self.endpoint else { return }
        let endpoint = self.endpoint
        self.working = true
        defer { self.working = false }
        do {
            let client = DaemonClient(endpoint: endpoint.endpoint)
            if resume {
                _ = try await client.resumeBackgroundJob(job.id)
            } else {
                _ = try await client.cancelBackgroundJob(job.id)
            }
            guard EndpointStore.shared.selection == endpoint else { return }
            await self.refresh()
        } catch {
            guard EndpointStore.shared.selection == endpoint else { return }
            self.error = error.localizedDescription
        }
    }

    private func restart() async {
        guard !self.working, self.endpoint == EndpointStore.shared.selection,
              self.endpoint.endpoint.canRestartRuntime else { return }
        self.working = true
        defer { self.working = false }
        let endpoint = self.endpoint
        do {
            try await BackendService.shared.userRestart()
            guard EndpointStore.shared.selection == endpoint else { return }
            await self.refresh()
        } catch {
            guard EndpointStore.shared.selection == endpoint else { return }
            self.error = error.localizedDescription
        }
    }

    private func statusLabel(_ status: AgentJobStatusWire) -> String {
        switch status {
        case .queued: L10n.tr("settings.agent.jobs.status.queued")
        case .running: L10n.tr("settings.agent.jobs.status.running")
        case .cancelRequested: L10n.tr("settings.agent.jobs.status.cancel_requested")
        case .completed: L10n.tr("settings.agent.jobs.status.completed")
        case .failed: L10n.tr("settings.agent.jobs.status.failed")
        case .cancelled: L10n.tr("settings.agent.jobs.status.cancelled")
        case .interrupted: L10n.tr("settings.agent.jobs.status.interrupted")
        case .awaitingInput: L10n.tr("settings.agent.jobs.status.awaiting_input")
        }
    }

    private func jobStatus(_ status: AgentJobStatusWire) -> some View {
        HStack(spacing: 5) {
            if status == .running {
                BobeSpinner(size: 10)
            } else {
                Image(systemName: status == .completed ? "checkmark.circle"
                    : status == .awaitingInput ? "questionmark.bubble" : "circle")
            }
            Text(self.statusLabel(status))
        }
        .bobeTextStyle(.helper)
        .foregroundStyle(status == .completed ? self.theme.colors.success
            : status == .failed ? self.theme.colors.error : self.theme.colors.textMuted)
        .padding(.horizontal, 8)
        .padding(.vertical, 4)
        .background(Capsule().fill(self.theme.colors.border.opacity(0.25)))
    }

    private func toolDescription(_ tool: AgentToolWire) -> String {
        switch tool {
        case .read: L10n.tr("settings.agent.tools.read")
        case .bash: L10n.tr("settings.agent.tools.bash")
        case .edit: L10n.tr("settings.agent.tools.edit")
        case .write: L10n.tr("settings.agent.tools.write")
        case .grep: L10n.tr("settings.agent.tools.grep")
        case .find: L10n.tr("settings.agent.tools.find")
        case .ls: L10n.tr("settings.agent.tools.ls")
        case .subagent: L10n.tr("settings.agent.tools.subagent")
        case .backgroundJobs: L10n.tr("settings.agent.tools.background_jobs")
        }
    }

    private func toolLabel(_ tool: AgentToolWire) -> String {
        switch tool {
        case .read: L10n.tr("settings.providers.tool.read")
        case .bash: L10n.tr("settings.providers.tool.bash")
        case .edit: L10n.tr("settings.providers.tool.edit")
        case .write: L10n.tr("settings.providers.tool.write")
        case .grep: L10n.tr("settings.providers.tool.grep")
        case .find: L10n.tr("settings.providers.tool.find")
        case .ls: L10n.tr("settings.providers.tool.ls")
        case .subagent: L10n.tr("settings.providers.tool.subagent")
        case .backgroundJobs: L10n.tr("settings.providers.tool.background_jobs")
        }
    }
}
