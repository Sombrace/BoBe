import SwiftUI

struct ProviderProfilesSection: View {
    @Environment(\.theme) private var theme
    @State private var snapshot: ProvidersResponse?
    @State private var endpoint = EndpointStore.shared.selection
    @State private var editing: ProviderWorkload?
    @State private var adding = false
    @State private var editingProfile: String?
    @State private var connectionFields = ProviderConfigurationFields()
    @State private var apiKey = ""
    @State private var working = false
    @State private var error: String?
    @State private var probe: ProviderProbeResponse?
    @State private var testedWorkload: ProviderWorkload?
    @State private var testing: ProviderWorkload?
    @State private var showWorkloads = false
    @State private var confirmRemoval: ProviderProfileWire?
    @State private var accounts: [String: AuthStatusResponse] = [:]
    @State private var accountErrors: [String: String] = [:]
    @Namespace private var identity

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            if let snapshot {
                self.routeRow(.chat, snapshot: snapshot)
                BobeDisclosure(
                    L10n.tr("settings.providers.workload_overrides"),
                    isExpanded: self.$showWorkloads
                ) {
                    VStack(spacing: 12) {
                        ForEach(ProviderWorkload.allCases.filter { $0 != .chat }) { workload in
                            self.routeRow(workload, snapshot: snapshot)
                        }
                    }
                    .padding(.top, 12)
                }
                if snapshot.restartRequired {
                    HStack(spacing: 8) {
                        Image(systemName: "arrow.clockwise")
                        Text(L10n.tr("settings.providers.pending")).bobeTextStyle(.helper)
                        Spacer()
                        if self.endpoint.endpoint.canRestartRuntime {
                            Button(L10n.tr("settings.providers.apply")) { Task { await self.restart() } }
                                .bobeButton(.primary, size: .small)
                        }
                    }
                    .foregroundStyle(self.theme.colors.warning)
                    .help(L10n.tr(self.endpoint.endpoint.canRestartRuntime
                            ? "settings.agent.tools.restart" : "settings.providers.restart.external"))
                }
                BobeHairline()
                HStack {
                    Text(L10n.tr("settings.providers.connections")).bobeTextStyle(.rowTitle)
                    Spacer()
                    Button {
                        self.connectionFields = ProviderConfigurationFields()
                        self.editingProfile = nil
                        self.accounts = [:]
                        self.accountErrors = [:]
                        self.apiKey = ""
                        withAnimation(BobeMotion.disclose) { self.adding.toggle() }
                    } label: {
                        Image(systemName: self.adding ? "xmark" : "plus")
                    }
                    .bobeButton(.ghost, size: .small)
                    .disabled(self.working)
                    .accessibilityLabel(L10n.tr(self.adding ? "settings.editor.action.cancel" : "settings.providers.add"))
                    .help(L10n.tr(self.adding ? "settings.editor.action.cancel" : "settings.providers.add"))
                }
                ForEach(snapshot.profiles) { profile in
                    if self.adding && self.editingProfile == profile.id {
                        self.connectionEditor
                            .id(profile.id)
                    } else {
                        self.connectionRow(profile, snapshot: snapshot)
                    }
                }
                if self.adding && self.editingProfile == nil {
                    self.connectionEditor.id("new")
                }
            } else if self.error == nil {
                BobeSpinner(size: 16).frame(maxWidth: .infinity, minHeight: 80)
            }
            if self.working && self.testing == nil { BobeSpinner(size: 12) }
            if let error {
                HStack(alignment: .top) {
                    SettingsErrorBanner(message: error)
                    Button(L10n.tr("setup.cloud_auth.retry")) { Task { await self.load() } }
                        .bobeButton(.secondary, size: .small)
                }
            }
        }
        .disabled(self.working && !self.adding)
        .animation(BobeMotion.disclose, value: self.adding)
        .animation(BobeMotion.disclose, value: self.connectionFields.profileID)
        .task(id: self.endpoint) { await self.load() }
        .onReceive(NotificationCenter.default.publisher(for: .bobeEndpointIdentityChanged)) { _ in
            let current = EndpointStore.shared.selection
            if current.profileID != self.endpoint.profileID || current.authorityID != self.endpoint.authorityID {
                self.adding = false
                self.apiKey = ""
                self.snapshot = nil
                self.editingProfile = nil
            }
            self.endpoint = current
            self.editing = nil
            self.confirmRemoval = nil
            self.probe = nil
            self.error = nil
        }
    }

    private var connectionEditor: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text(self.connectionFields.isConfigured ? self.connectionFields.profileID : L10n.tr("settings.providers.add"))
                    .bobeTextStyle(.rowTitle)
                Spacer()
                Button {
                    withAnimation(BobeMotion.disclose) { self.adding = false }
                    self.apiKey = ""
                } label: {
                    Image(systemName: "xmark")
                }
                .bobeButton(.ghost, size: .small)
                .disabled(self.working)
                .accessibilityLabel(L10n.tr("settings.editor.action.cancel"))
            }
            ProviderConfigurationForm(
                fields: self.$connectionFields,
                apiKey: self.$apiKey,
                showsModelSelection: false,
                onCriticalOperationChange: { self.working = $0 },
                onConnected: {
                    withAnimation(BobeMotion.disclose) { self.adding = false }
                    Task { await self.load() }
                },
                identity: self.editingProfile == nil ? nil : self.identity
            )
        }
        .padding(16)
        .background(RoundedRectangle(cornerRadius: 16).fill(self.theme.colors.surface))
        .overlay(RoundedRectangle(cornerRadius: 16).stroke(self.theme.colors.border.opacity(0.6), lineWidth: 1))
        .transition(.opacity.combined(with: .scale(scale: 0.98, anchor: .top)))
    }

    private func routeRow(_ workload: ProviderWorkload, snapshot: ProvidersResponse) -> some View {
        let endpoint = self.endpoint
        let route = snapshot.routes.route(for: workload)
        let active = snapshot.activeRoutes.route(for: workload)
        let provider = snapshot.profiles.first { $0.id == route.profile }
        return VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 10) {
                Image(systemName: workload.symbol)
                    .foregroundStyle(self.theme.colors.primary)
                    .frame(width: 22)
                    .accessibilityHidden(true)
                Text(workload.label).bobeTextStyle(.settingsBody)
                Spacer(minLength: 12)
                if snapshot.isPending(workload) {
                    Image(systemName: "clock")
                        .foregroundStyle(self.theme.colors.warning)
                        .help(L10n.tr("settings.providers.running_model", active.model ?? "—"))
                        .accessibilityLabel(L10n.tr("settings.providers.pending"))
                }
                Button {
                    self.editing = workload
                } label: {
                    HStack(spacing: 6) {
                        VStack(alignment: .trailing, spacing: 2) {
                            Text(route.model ?? L10n.tr("settings.providers.choose_model"))
                                .lineLimit(1).truncationMode(.middle)
                            Text(provider?.kind.displayName ?? route.profile)
                                .bobeTextStyle(.helper)
                                .foregroundStyle(self.theme.colors.textMuted)
                        }
                        Image(systemName: "chevron.up.chevron.down")
                            .font(.system(size: 10))
                            .foregroundStyle(self.theme.colors.textMuted)
                    }
                }
                .buttonStyle(.plain)
                .accessibilityLabel("\(workload.label), \(route.model ?? L10n.tr("settings.providers.choose_model"))")
                .popover(isPresented: Binding(
                    get: { self.editing == workload },
                    set: { if !$0 { self.editing = nil } }
                ), arrowEdge: .bottom) {
                    ProviderModelPicker(
                        profiles: snapshot.profiles,
                        initial: route,
                        selection: endpoint
                    ) { route, _ in
                        self.editing = nil
                        Task { await self.saveRoute(workload, route: route, endpoint: endpoint) }
                    }
                }
                Button {
                    Task { await self.test(workload, route: route, endpoint: endpoint) }
                } label: {
                    if self.testing == workload {
                        BobeSpinner(size: 16)
                    } else {
                        Image(systemName: self.testedWorkload == workload && self.probe?.profile == route.profile && self.probe?.model == route.model
                            ? (self.probe?.ok == true ? "checkmark.circle" : "exclamationmark.circle") : "play.circle")
                            .contentTransition(BobeMotion.symbolReplace)
                    }
                }
                .buttonStyle(.plain)
                .help(L10n.tr("settings.providers.test"))
                .accessibilityLabel(L10n.tr("settings.providers.test"))
                .disabled(provider?.restartRequired != false || route.model == nil)
            }
            if self.testedWorkload == workload, let probe {
                HStack(spacing: 12) {
                    self.probeCheck("sparkles", label: L10n.tr("settings.providers.check_model"), passed: probe.modelExecution)
                    self.probeCheck("waveform", label: L10n.tr("settings.providers.check_stream"), passed: probe.streaming)
                    self.probeCheck("wrench", label: L10n.tr("settings.providers.check_tools"), passed: probe.toolExecution)
                    Spacer()
                }
                .padding(.leading, 32)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .animation(BobeMotion.disclose, value: self.testedWorkload)
        .disabled(self.working)
    }

    private func probeCheck(_ symbol: String, label: String, passed: Bool) -> some View {
        Label(label, systemImage: passed ? "checkmark.circle.fill" : symbol)
            .bobeTextStyle(.helper)
            .foregroundStyle(passed ? self.theme.colors.success : self.theme.colors.error)
    }

    private func connectionRow(_ profile: ProviderProfileWire, snapshot: ProvidersResponse) -> some View {
        let endpoint = self.endpoint
        return HStack(spacing: 10) {
            Button {
                self.connectionFields.load(snapshot, profileID: profile.id)
                self.editingProfile = profile.id
                self.apiKey = ""
                withAnimation(BobeMotion.disclose) { self.adding = true }
            } label: {
                HStack(spacing: 10) {
                    ProviderMark(kind: profile.kind)
                        .matchedGeometryEffect(id: profile.id, in: self.identity)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(profile.kind.displayName).bobeTextStyle(.settingsBody)
                        Text(profile.id).bobeTextStyle(.helper).foregroundStyle(self.theme.colors.textMuted)
                    }
                    Spacer()
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            if profile.restartRequired {
                Image(systemName: "clock").foregroundStyle(self.theme.colors.warning)
                    .help(L10n.tr("settings.providers.pending"))
            } else if let error = self.accountErrors[profile.id] {
                Image(systemName: "exclamationmark.circle")
                    .foregroundStyle(self.theme.colors.error)
                    .help(error)
                    .accessibilityLabel(error)
            } else {
                let available = profile.authMethod == .oauth
                    ? self.accounts[profile.id]?.isAuthenticated == true
                    : profile.credentialPresent || profile.offline
                Image(systemName: available ? "key.fill" : "key.slash")
                    .foregroundStyle(self.theme.colors.textMuted)
                    .help(self.accounts[profile.id]?.login ?? L10n.tr(available
                            ? "settings.providers.accessibility.credential_ready"
                            : "settings.providers.accessibility.credential_missing"))
            }
            if self.confirmRemoval?.id == profile.id {
                Button(L10n.tr("settings.providers.remove"), role: .destructive) {
                    self.confirmRemoval = nil
                    Task { await self.accountAction(profile, action: .remove, endpoint: endpoint) }
                }
                .bobeButton(.destructive, size: .small)
                Button {
                    withAnimation(BobeMotion.disclose) { self.confirmRemoval = nil }
                } label: { Image(systemName: "xmark") }
                    .bobeButton(.ghost, size: .small)
                    .accessibilityLabel(L10n.tr("settings.editor.action.cancel"))
            } else {
                Menu {
                    Button(L10n.tr("settings.providers.reconnect")) {
                        self.connectionFields.load(snapshot, profileID: profile.id)
                        self.editingProfile = profile.id
                        self.apiKey = ""
                        withAnimation(BobeMotion.disclose) { self.adding = true }
                    }
                    if profile.authMethod == .oauth {
                        Button(L10n.tr("settings.providers.sign_out")) {
                            Task { await self.accountAction(profile, action: .signOut, endpoint: endpoint) }
                        }.disabled(profile.restartRequired)
                    } else if profile.credentialPresent {
                        Button(L10n.tr("settings.providers.clear_key"), role: .destructive) {
                            Task { await self.accountAction(profile, action: .clearKey, endpoint: endpoint) }
                        }
                    }
                    BobeHairline()
                    Button(L10n.tr("settings.providers.remove"), role: .destructive) {
                        withAnimation(BobeMotion.disclose) { self.confirmRemoval = profile }
                    }
                    .disabled(!snapshot.canRemove(profile))
                    .help(L10n.tr("settings.providers.remove.help"))
                } label: {
                    Image(systemName: "ellipsis")
                }
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()
                .accessibilityLabel(L10n.tr("settings.providers.account_actions"))
            }
        }
        .padding(12)
        .background(RoundedRectangle(cornerRadius: 14).fill(self.theme.colors.surface))
        .animation(BobeMotion.disclose, value: self.confirmRemoval?.id)
        .disabled(self.working)
    }

    private func load() async {
        let endpoint = self.endpoint
        do {
            let response = try await DaemonClient(endpoint: endpoint.endpoint).listProviders()
            guard !Task.isCancelled, EndpointStore.shared.selection == endpoint else { return }
            if self.snapshot?.routes != response.routes || self.snapshot?.profiles != response.profiles {
                self.probe = nil
                self.testedWorkload = nil
            }
            self.snapshot = response
            self.error = nil
            for profile in response.profiles where profile.authMethod == .oauth && !profile.restartRequired {
                do {
                    let auth = try await DaemonClient(endpoint: endpoint.endpoint).getAuthStatus(profile: profile.id)
                    guard !Task.isCancelled, EndpointStore.shared.selection == endpoint else { return }
                    self.accounts[profile.id] = auth
                    self.accountErrors[profile.id] = nil
                } catch {
                    guard !Task.isCancelled, EndpointStore.shared.selection == endpoint else { return }
                    self.accounts[profile.id] = nil
                    self.accountErrors[profile.id] = error.localizedDescription
                }
            }
        } catch {
            guard !Task.isCancelled, EndpointStore.shared.selection == endpoint else { return }
            self.error = error.localizedDescription
        }
    }

    private func perform(endpoint: EndpointSelection, _ operation: (DaemonClient) async throws -> Void) async {
        guard !self.working, EndpointStore.shared.selection == endpoint, self.endpoint == endpoint else { return }
        self.working = true
        self.error = nil
        defer { self.working = false }
        do {
            try await operation(DaemonClient(endpoint: endpoint.endpoint))
            guard EndpointStore.shared.selection == endpoint else { return }
            await self.load()
        } catch {
            guard EndpointStore.shared.selection == endpoint else { return }
            self.error = error.localizedDescription
        }
    }

    private func saveRoute(_ workload: ProviderWorkload, route: WorkloadRouteWire, endpoint: EndpointSelection) async {
        self.probe = nil
        await self.perform(endpoint: endpoint) { client in _ = try await client.setProviderRoute(workload, route: route) }
    }

    private func test(_ workload: ProviderWorkload, route: WorkloadRouteWire, endpoint: EndpointSelection) async {
        self.testing = workload
        self.testedWorkload = nil
        defer { self.testing = nil }
        await self.perform(endpoint: endpoint) { client in
            let result = try await client.testProvider(route)
            guard EndpointStore.shared.selection == endpoint else { return }
            self.probe = result
            self.testedWorkload = workload
            if !result.ok { throw ProviderConnectionError.failed(result.error ?? L10n.tr("settings.providers.test_failed")) }
        }
    }

    private enum AccountAction { case signOut, clearKey, remove }

    private func accountAction(_ profile: ProviderProfileWire, action: AccountAction, endpoint: EndpointSelection) async {
        self.probe = nil
        await self.perform(endpoint: endpoint) { client in
            switch action {
            case .signOut: try await client.logoutProvider(profile.id)
            case .clearKey: _ = try await client.clearProviderCredential(profile.id)
            case .remove: _ = try await client.removeProvider(profile.id)
            }
        }
    }

    private func restart() async {
        await self.perform(endpoint: self.endpoint) { _ in try await BackendService.shared.userRestart() }
    }
}

enum ProviderConnectionError: LocalizedError {
    case failed(String)
    var errorDescription: String? {
        switch self {
        case let .failed(message): message
        }
    }
}
