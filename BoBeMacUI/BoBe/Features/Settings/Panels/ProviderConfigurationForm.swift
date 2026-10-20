import SwiftUI

private struct ProviderSignInContext: Identifiable {
    let id = UUID()
    let profile: String
    let providerName: String
    let selection: EndpointSelection
}

struct ProviderConfigurationForm: View {
    @Binding var fields: ProviderConfigurationFields
    @Binding var apiKey: String
    var showsModelSelection = true
    var onCriticalOperationChange: (Bool) -> Void = { _ in }
    var onConnected: () -> Void = {}
    var onConnectionStateChange: (Bool) -> Void = { _ in }
    var identity: Namespace.ID?

    @Environment(\.theme) private var theme
    @State private var isPreparing = false
    @State private var connected = false
    @State private var error: String?
    @State private var login: String?
    @State private var catalog: [ProviderModelInfo] = []
    @State private var choosingModel = false
    @State private var signIn: ProviderSignInContext?
    @State private var observedSelection = EndpointStore.shared.selection
    @State private var showsAdvanced = false
    @Namespace private var kindSelection

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            self.kindChooser
            if self.fields.kind.supportsOAuth, self.fields.kind != .copilot {
                BobeSegmented(
                    selection: Binding(
                        get: { self.fields.authMethod },
                        set: {
                            self.apiKey = ""
                            self.fields.selectAuthMethod($0)
                            self.invalidate()
                        }
                    ),
                    options: [ProviderAuthMethodWire.apiKey, ProviderAuthMethodWire.oauth],
                    title: { method in
                        L10n.tr(method == .oauth ? "settings.providers.auth.oauth" : "settings.providers.auth.api_key")
                    }
                )
                .accessibilityLabel(L10n.tr("settings.providers.auth.method"))
                .disabled(self.kindLocked)
            }
            self.accessFields
            self.advancedFields
            self.connectionRow
            if self.connected, self.showsModelSelection {
                self.modelRow
            }
            if let error {
                SettingsErrorBanner(message: error)
                    .transition(.bobeRise)
            }
        }
        .disabled(self.isPreparing)
        .animation(BobeMotion.standard, value: self.connected)
        .animation(BobeMotion.standard, value: self.fields.kind)
        .animation(BobeMotion.standard, value: self.fields.authMethod)
        .animation(BobeMotion.standard, value: self.showsAdvanced)
        .animation(BobeMotion.standard, value: self.error)
        .onChange(of: self.fields.baseURL) { _, _ in self.invalidate() }
        .onChange(of: self.fields.profileID) { _, _ in self.invalidate() }
        .onChange(of: self.fields.organization) { _, _ in self.invalidate() }
        .onChange(of: self.fields.offline) { _, _ in self.invalidate() }
        .onChange(of: self.apiKey) { _, value in
            if !value.isEmpty { self.connected = false }
        }
        .task {
            guard self.showsModelSelection, self.fields.isConfigured else { return }
            let selection = self.observedSelection
            self.isPreparing = true
            defer { self.isPreparing = false }
            do { try await self.completeConnection(selection) }
            catch {
                guard !Task.isCancelled, EndpointStore.shared.selection == selection else { return }
                self.error = error.localizedDescription
            }
        }
        .sheet(item: self.$signIn) { context in
            ProviderSignInSheet(
                profile: context.profile,
                providerName: context.providerName,
                selection: context.selection,
                onSuccess: { self.finishSignIn(context) },
                onClose: { self.signIn = nil }
            )
        }
        .onChange(of: self.isPreparing || self.signIn != nil) { _, critical in
            self.onCriticalOperationChange(critical)
        }
        .onChange(of: self.connected) { _, ready in self.onConnectionStateChange(ready) }
        .onDisappear {
            self.onCriticalOperationChange(false)
            self.onConnectionStateChange(false)
        }
        .onReceive(NotificationCenter.default.publisher(for: .bobeEndpointIdentityChanged)) { _ in
            let current = EndpointStore.shared.selection
            let changed = current.profileID != self.observedSelection.profileID
                || current.authorityID != self.observedSelection.authorityID
            self.observedSelection = current
            guard changed else { return }
            self.apiKey = ""
            self.fields = ProviderConfigurationFields()
            self.signIn = nil
            self.invalidate()
        }
    }

    private var kindLocked: Bool {
        self.fields.isConfigured && !self.showsModelSelection
    }

    @ViewBuilder
    private var kindChooser: some View {
        if self.kindLocked {
            HStack(spacing: 12) {
                if let identity {
                    ProviderMark(kind: self.fields.kind, size: 40)
                        .matchedGeometryEffect(id: self.fields.profileID, in: identity)
                } else {
                    ProviderMark(kind: self.fields.kind, size: 40)
                }
                Text(self.fields.kind.displayName)
                    .bobeTextStyle(.rowTitle)
                    .foregroundStyle(self.theme.colors.text)
                Spacer(minLength: 0)
            }
        } else {
            LazyVGrid(
                columns: [GridItem(.adaptive(minimum: 104), spacing: 8)],
                alignment: .leading,
                spacing: 8
            ) {
                ForEach(ProviderKindWire.choices, id: \.self) { kind in
                    ProviderKindTile(
                        kind: kind,
                        isSelected: kind == self.fields.kind,
                        selection: self.kindSelection
                    ) {
                        guard kind != self.fields.kind else { return }
                        self.apiKey = ""
                        self.fields.selectKind(kind)
                        self.invalidate()
                    }
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(L10n.tr("settings.providers.direct.kind"))
        }
    }

    @ViewBuilder
    private var accessFields: some View {
        if self.fields.needsBaseURL {
            TextField(L10n.tr("settings.providers.base_url"), text: self.$fields.baseURL)
                .bobeField()
                .help(L10n.tr("settings.providers.endpoint.help"))
                .accessibilityIdentifier("provider.base-url")
                .transition(.bobeRise)
        }
        if self.fields.authMethod == .apiKey, !(self.fields.kind == .openAICompatible && self.fields.offline) {
            SecureField(
                L10n.tr(self.fields.credentialPresent ? "settings.providers.replace_key" : "settings.providers.key_placeholder"),
                text: self.$apiKey
            )
            .bobeField()
            .help(L10n.tr("settings.providers.key_help"))
            .accessibilityIdentifier("provider.api-key")
            .transition(.bobeRise)
        }
        if self.fields.kind == .openAICompatible {
            HStack(spacing: 10) {
                Text(L10n.tr("settings.providers.no_key"))
                    .bobeTextStyle(.settingsBody)
                    .foregroundStyle(self.theme.colors.text)
                Spacer(minLength: 0)
                BobeToggle(isOn: self.$fields.offline, accessibilityLabel: L10n.tr("settings.providers.no_key"))
            }
            .padding(.horizontal, 4)
            .onChange(of: self.fields.offline) { _, value in
                if !value { self.fields.localRuntime = nil }
            }
            .transition(.bobeRise)
        }
    }

    private var advancedFields: some View {
        VStack(alignment: .leading, spacing: 10) {
            Button {
                self.showsAdvanced.toggle()
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .bold))
                        .rotationEffect(.degrees(self.showsAdvanced ? 90 : 0))
                    Text(L10n.tr("settings.providers.advanced"))
                        .bobeTextStyle(.helper)
                }
                .foregroundStyle(self.theme.colors.textMuted)
                .padding(.vertical, 4)
                .padding(.horizontal, 4)
                .contentShape(Rectangle())
            }
            .buttonStyle(BobePressStyle())
            .accessibilityIdentifier("provider.advanced")
            .bobeExpansionValue(self.showsAdvanced)
            if self.showsAdvanced {
                VStack(alignment: .leading, spacing: 8) {
                    self.advancedField(L10n.tr("settings.providers.profile")) {
                        TextField("openai", text: self.$fields.profileID)
                            .bobeField()
                            .disabled(self.fields.isConfigured)
                    }
                    if self.fields.kind == .openAI, self.fields.authMethod == .apiKey {
                        self.advancedField(L10n.tr("settings.providers.organization")) {
                            TextField(L10n.tr("settings.providers.optional"), text: self.$fields.organization)
                                .bobeField()
                        }
                    }
                }
                .transition(.bobeRise)
            }
        }
    }

    private func advancedField(_ title: String, @ViewBuilder field: () -> some View) -> some View {
        HStack(spacing: 12) {
            Text(title)
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.textMuted)
                .frame(width: 96, alignment: .leading)
            field()
        }
    }

    private var connectionRow: some View {
        HStack(spacing: 10) {
            if self.connected {
                HStack(spacing: 7) {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: 14, weight: .semibold))
                    Text(self.login ?? L10n.tr("settings.providers.connected"))
                        .bobeTextStyle(.rowMeta)
                        .lineLimit(1)
                }
                .foregroundStyle(self.theme.colors.success)
                .padding(.horizontal, 12)
                .frame(height: 32)
                .background(Capsule(style: .circular).fill(self.theme.colors.success.opacity(0.12)))
                .transition(.bobePop(anchor: .leading))
                .accessibilityIdentifier("provider.connected")
            } else {
                Button {
                    Task { await self.connect() }
                } label: {
                    HStack(spacing: 8) {
                        if self.isPreparing {
                            BobeSpinner(size: 12)
                        }
                        Text(L10n.tr(self.fields.authMethod == .oauth ? "settings.providers.auth.oauth" : "settings.providers.connect"))
                    }
                }
                .bobeButton(.primary, size: .regular)
                .disabled(!self.fields.canConnect(apiKey: self.apiKey) || self.isPreparing)
                .accessibilityIdentifier("provider.connect")
                .transition(.bobePop(anchor: .leading))
            }
            Spacer(minLength: 0)
        }
    }

    private var modelRow: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                self.choosingModel = true
            } label: {
                HStack(spacing: 10) {
                    BobeIconTile(systemName: "sparkles", tint: self.theme.colors.primary, size: 28)
                    Text(L10n.tr("settings.providers.workload.chat"))
                        .bobeTextStyle(.settingsBody)
                        .foregroundStyle(self.theme.colors.text)
                    Spacer(minLength: 8)
                    Text(self.fields.chat.isEmpty ? L10n.tr("settings.providers.choose_model") : self.fields.chat)
                        .bobeTextStyle(.rowMeta)
                        .foregroundStyle(self.fields.chat.isEmpty ? self.theme.colors.primary : self.theme.colors.textMuted)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Image(systemName: "chevron.up.chevron.down")
                        .font(.system(size: 10, weight: .semibold))
                        .foregroundStyle(self.theme.colors.textMuted)
                }
                .padding(.horizontal, 12)
                .frame(height: 48)
                .bobeRaised(cornerRadius: BobeRadius.control)
                .contentShape(RoundedRectangle(cornerRadius: BobeRadius.control, style: .continuous))
            }
            .buttonStyle(BobePressStyle(scale: 0.985))
            .accessibilityIdentifier("provider.chat-model")
            .popover(isPresented: self.$choosingModel) {
                ProviderModelPicker(
                    profiles: [self.profile],
                    initial: WorkloadRouteWire(profile: self.fields.profileID, model: self.fields.chat, reasoningEffort: nil),
                    selection: self.observedSelection,
                    showsReasoning: false
                ) { _, model in
                    self.fields.selectModel(model)
                    self.choosingModel = false
                }
            }
        }
        .transition(.bobeRise)
    }

    private var profile: ProviderProfileWire {
        ProviderProfileWire(
            id: self.fields.profileID, kind: self.fields.kind,
            authMethod: self.fields.authMethod, baseURL: self.fields.baseURL,
            offline: self.fields.offline, credentialPresent: self.fields.credentialPresent
        )
    }

    private func invalidate() {
        self.connected = false
        self.catalog = []
        self.login = nil
        self.error = nil
    }

    private func connect() async {
        guard !self.isPreparing, self.fields.canConnect(apiKey: self.apiKey),
              EndpointStore.shared.selection == self.observedSelection else { return }
        let selection = EndpointStore.shared.selection
        var submitted = self.fields
        let key = self.apiKey.trimmingCharacters(in: .whitespacesAndNewlines)
        self.isPreparing = true
        self.error = nil
        defer { self.isPreparing = false }
        do {
            let client = DaemonClient(endpoint: selection.endpoint)
            if !submitted.isConfigured {
                let providers = try await client.listProviders()
                guard EndpointStore.shared.selection == selection else { return }
                submitted.profileID = ProviderConfigurationFields.availableID(
                    submitted.profileID, existing: Set(providers.profiles.map(\.id))
                )
                self.fields.profileID = submitted.profileID
            }
            _ = try await client.upsertProvider(submitted.profileID, request: ProviderUpsertRequest(submitted.activation(apiKey: "")))
            guard EndpointStore.shared.selection == selection else { return }
            if submitted.authMethod == .apiKey, !key.isEmpty {
                _ = try await client.setProviderCredential(submitted.profileID, apiKey: key)
                guard EndpointStore.shared.selection == selection else { return }
                self.fields.credentialPresent = true
                if self.apiKey.trimmingCharacters(in: .whitespacesAndNewlines) == key { self.apiKey = "" }
            }
            self.fields.isConfigured = true
            let saved = try await client.listProviders()
            guard EndpointStore.shared.selection == selection else { return }
            if saved.profiles.first(where: { $0.id == submitted.profileID })?.restartRequired == true {
                if !self.showsModelSelection {
                    self.onConnected()
                    return
                }
                guard selection.endpoint.canRestartRuntime else { throw ProviderSetupError.externalRestartRequired }
                try await BackendService.shared.userRestart()
            }
            let current = EndpointStore.shared.selection
            guard current.profileID == selection.profileID, current.authorityID == selection.authorityID else { return }
            self.observedSelection = current
            if submitted.authMethod == .oauth {
                self.signIn = ProviderSignInContext(profile: submitted.profileID, providerName: submitted.kind.displayName, selection: current)
            } else {
                try await self.completeConnection(current)
            }
        } catch {
            guard EndpointStore.shared.selection.profileID == selection.profileID,
                  EndpointStore.shared.selection.authorityID == selection.authorityID else { return }
            self.error = error.localizedDescription
        }
    }

    private func completeConnection(_ selection: EndpointSelection) async throws {
        let client = DaemonClient(endpoint: selection.endpoint)
        let profile = self.fields.profileID
        let result = try await client.listModels(profile: profile)
        guard EndpointStore.shared.selection == selection, self.fields.profileID == profile else { return }
        self.catalog = result.models
        if self.fields.authMethod == .oauth {
            let status = try await client.getAuthStatus(profile: profile)
            guard EndpointStore.shared.selection == selection else { return }
            self.fields.isAuthenticated = status.isAuthenticated
            self.login = status.login
            guard status.isAuthenticated else { throw ProviderConnectionError.failed(L10n.tr("setup.cloud_auth.signed_out_hint")) }
        }
        self.connected = true
        if self.showsModelSelection {
            if !self.catalog.contains(where: { $0.id == self.fields.chat }) {
                self.fields.chat = ""
            }
            if self.fields.chat.isEmpty, self.catalog.count == 1, let only = self.catalog.first {
                self.fields.selectModel(only)
            }
        }
        self.onConnected()
    }

    private func finishSignIn(_ context: ProviderSignInContext) {
        self.signIn = nil
        guard EndpointStore.shared.selection == context.selection else { return }
        self.isPreparing = true
        Task {
            defer { self.isPreparing = false }
            do { try await self.completeConnection(context.selection) }
            catch {
                guard EndpointStore.shared.selection == context.selection else { return }
                self.error = error.localizedDescription
            }
        }
    }
}

private struct ProviderKindTile: View {
    let kind: ProviderKindWire
    let isSelected: Bool
    let selection: Namespace.ID
    let action: () -> Void

    @Environment(\.theme) private var theme
    @Environment(\.isEnabled) private var isEnabled
    @State private var isHovered = false

    var body: some View {
        Button(action: self.action) {
            VStack(spacing: 7) {
                ProviderMark(kind: self.kind, size: 30)
                Text(self.kind.displayName)
                    .bobeTextStyle(.caption)
                    .foregroundStyle(self.isSelected ? self.theme.colors.text : self.theme.colors.textMuted)
                    .multilineTextAlignment(.center)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(height: 28, alignment: .top)
            }
            .frame(maxWidth: .infinity)
            .padding(.top, 10)
            .padding(.bottom, 4)
            .padding(.horizontal, 6)
            .background {
                if self.isSelected {
                    RoundedRectangle(cornerRadius: BobeRadius.control, style: .continuous)
                        .fill(self.theme.colors.surface)
                        .shadow(color: self.theme.shadowTint.opacity(0.14), radius: 8, x: 0, y: 4)
                        .overlay(
                            RoundedRectangle(cornerRadius: BobeRadius.control, style: .continuous)
                                .strokeBorder(self.theme.colors.primary.opacity(0.6), lineWidth: 1.5)
                        )
                        .matchedGeometryEffect(id: "provider.kind.selection", in: self.selection)
                } else {
                    RoundedRectangle(cornerRadius: BobeRadius.control, style: .continuous)
                        .fill(self.theme.well.opacity(self.isHovered ? 1 : 0.55))
                }
            }
            .contentShape(RoundedRectangle(cornerRadius: BobeRadius.control, style: .continuous))
            .opacity(self.isEnabled ? 1 : 0.5)
        }
        .buttonStyle(BobePressStyle())
        .onHover { self.isHovered = $0 }
        .animation(BobeMotion.fade, value: self.isHovered)
        .accessibilityLabel(self.kind.displayName)
        .accessibilityAddTraits(self.isSelected ? .isSelected : [])
        .accessibilityIdentifier("provider.kind.\(self.kind.rawValue)")
    }
}
