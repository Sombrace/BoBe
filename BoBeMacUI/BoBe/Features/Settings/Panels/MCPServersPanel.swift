import SwiftUI

struct MCPServersPanel: View {
    @State private var rawJson = ""
    @State private var servers: [MCPServer] = []
    @State private var isLoading = false
    @State private var isValidating = false
    @State private var isSaving = false
    @State private var status: String?
    @State private var error: String?
    @State private var lastValidSecretMap: [String: [String]]?
    @Environment(ExpertMode.self) private var expertMode
    @Environment(SettingsStore.self) private var store
    @Environment(\.theme) private var theme

    private var isJsonEmpty: Bool {
        self.rawJson.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text(L10n.tr("settings.mcp.description"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)

                if !self.store.restartFields.isEmpty {
                    RestartRequiredBanner(
                        fields: self.store.restartFields,
                        onDismiss: { self.store.clearRestartFields() }
                    )
                }

                if let storeError = self.store.error {
                    SettingsErrorBanner(message: storeError)
                }
                if let savedMessage = self.store.savedMessage {
                    SettingsSavedToast(message: savedMessage)
                }

                self.enableToggleRow

                if !self.expertMode.isEnabled {
                    Text(L10n.tr("settings.mcp.novice.lede"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                        .padding(.bottom, 4)
                }

                if self.expertMode.isEnabled {
                    CodeEditor(text: self.$rawJson, theme: self.theme, fontSize: 12)
                        .frame(height: 260)
                        .bobeRaised(cornerRadius: 16)

                    HStack(spacing: 8) {
                        Button(L10n.tr("settings.mcp.action.reload")) { self.reloadConfig() }
                            .bobeButton(.secondary, size: .small)
                            .disabled(self.isLoading || self.isValidating || self.isSaving)

                        Button {
                            self.validateConfig()
                        } label: {
                            HStack(spacing: 4) {
                                if self.isValidating {
                                    BobeSpinner(size: 12)
                                }
                                Text(
                                    self.isValidating
                                        ? L10n.tr("settings.mcp.action.validating")
                                        : L10n.tr("settings.mcp.action.validate")
                                )
                            }
                        }
                        .bobeButton(.secondary, size: .small)
                        .disabled(self.isLoading || self.isValidating || self.isSaving || self.isJsonEmpty)

                        Button {
                            self.saveConfig()
                        } label: {
                            HStack(spacing: 4) {
                                if self.isSaving {
                                    BobeSpinner(size: 12)
                                }
                                Text(
                                    self.isSaving
                                        ? L10n.tr("settings.shared.action.saving")
                                        : L10n.tr("settings.shared.action.save")
                                )
                            }
                        }
                        .bobeButton(.primary, size: .small)
                        .disabled(self.isLoading || self.isValidating || self.isSaving || self.isJsonEmpty)

                        Spacer()

                        Button(L10n.tr("settings.mcp.action.reset")) { self.resetConfig() }
                            .bobeButton(.destructive, size: .small)
                            .disabled(self.isLoading || self.isValidating || self.isSaving)
                    }

                    if let status {
                        Text(status)
                            .bobeTextStyle(.helper)
                            .foregroundStyle(self.theme.colors.success)
                    }

                    if let error {
                        Text(error)
                            .bobeTextStyle(.helper)
                            .foregroundStyle(self.theme.colors.error)
                    }
                } else {
                    self.noviceEditorHidden
                }

                BobeHairline()

                self.discoverySection
            }
            .padding(.horizontal, 16)
            .padding(.top, 12)
            .padding(.bottom, 16)
        }
        .task {
            await self.loadConfig()
            await self.store.refresh()
        }
        .onDisappear { self.store.cancelToast() }
    }

    private var enableToggleRow: some View {
        SettingsRow(
            label: L10n.tr("settings.mcp.enable"),
            description: L10n.tr("settings.mcp.enable.description")
        ) {
            BobeToggle(
                isOn: self.mcpEnabledBinding,
                accessibilityLabel: L10n.tr("settings.mcp.enable")
            )
        }
    }

    private var mcpEnabledBinding: Binding<Bool> {
        self.store.binding(\.mcpEnabled, fallback: false)
    }

    private var noviceEditorHidden: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Image(systemName: "wrench.and.screwdriver")
                    .font(.system(size: 12))
                    .foregroundStyle(self.theme.colors.textMuted)
                Text(L10n.tr("settings.mcp.novice.editor_hidden"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
            }
            Button(L10n.tr("settings.window.search.enable_expert")) {
                self.expertMode.setEnabled(true)
            }
            .bobeButton(.secondary, size: .small)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .bobeRaised(cornerRadius: 16)
    }

    private var discoverySection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(L10n.tr("settings.mcp.discovery.title"))
                .font(.headline)
                .foregroundStyle(self.theme.colors.text)

            if self.isLoading {
                HStack(spacing: 8) {
                    BobeSpinner(size: 12)
                    Text(L10n.tr("settings.mcp.loading"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                }
            } else if self.servers.isEmpty {
                Text(L10n.tr("settings.mcp.discovery.empty"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
            } else {
                LazyVStack(alignment: .leading, spacing: 6) {
                    ForEach(self.servers) { server in
                        VStack(alignment: .leading, spacing: 4) {
                            HStack(spacing: 8) {
                                Text(server.name)
                                    .bobeTextStyle(.rowMeta)
                                    .fontWeight(.semibold)
                                Text(
                                    server.enabled
                                        ? L10n.tr("settings.mcp.discovery.status.enabled")
                                        : L10n.tr("settings.mcp.discovery.status.disabled")
                                )
                                .font(.system(size: 10, weight: .medium))
                                .foregroundStyle(server.enabled ? self.theme.colors.success : self.theme.colors.textMuted)
                                self.statusBadge(for: server)
                            }
                            .foregroundStyle(self.theme.colors.text)

                            if !server.command.isEmpty {
                                Text("\(server.command) \(server.args.joined(separator: " "))")
                                    .font(.system(size: 10, design: .monospaced))
                                    .foregroundStyle(self.theme.colors.textMuted)
                            }

                            if let serverError = server.error, !serverError.isEmpty {
                                Text(serverError)
                                    .font(.system(size: 10))
                                    .foregroundStyle(self.theme.colors.error)
                            }
                        }
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 6).fill(self.theme.colors.surface))
                    }

                    if self.servers.contains(where: { $0.status == nil }) {
                        Text(L10n.tr("settings.mcp.discovery.runtime_state_pending"))
                            .font(.system(size: 10).italic())
                            .foregroundStyle(self.theme.colors.textMuted.opacity(0.8))
                            .padding(.top, 4)
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func statusBadge(for server: MCPServer) -> some View {
        if let status = server.status {
            switch status {
            case McpServerStatusWire.connected:
                self.badge(
                    text: L10n.tr("settings.mcp.runtime.connected"),
                    color: self.theme.colors.success
                )
            case McpServerStatusWire.failed:
                self.badge(
                    text: L10n.tr("settings.mcp.runtime.failed"),
                    color: self.theme.colors.error
                )
            case McpServerStatusWire.pending:
                self.badge(
                    text: L10n.tr("settings.mcp.runtime.pending"),
                    color: self.theme.colors.textMuted
                )
            case McpServerStatusWire.stopped:
                EmptyView()
            default:
                self.badge(text: status, color: self.theme.colors.textMuted)
            }
        }
    }

    private func badge(text: String, color: Color) -> some View {
        Text(text)
            .font(.system(size: 9, weight: .medium))
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(
                Capsule()
                    .fill(color.opacity(0.15))
                    .overlay(Capsule().stroke(color.opacity(0.5), lineWidth: 0.5))
            )
            .foregroundStyle(color)
    }

    private func loadConfig() async {
        self.isLoading = true
        self.status = nil
        self.error = nil
        defer { self.isLoading = false }

        do {
            let response = try await DaemonClient.shared.getMCPConfig()
            self.rawJson = response.rawJson
            self.servers = response.servers
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func reloadConfig() {
        Task { await self.loadConfig() }
    }

    private func validateConfig() {
        guard !self.rawJson.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            self.error = L10n.tr("settings.mcp.error.empty_json")
            return
        }

        self.isValidating = true
        self.status = nil
        self.error = nil
        let capturedJson = self.rawJson
        let request = MCPConfigMutationRequest(
            rawJson: capturedJson,
            secretKeys: self.extractSecretKeyMap(from: capturedJson)
        )

        Task {
            defer { self.isValidating = false }
            do {
                let response = try await DaemonClient.shared.validateMCPConfig(request)
                if response.valid {
                    self.rawJson = response.normalizedJson
                    self.status = L10n.tr("settings.mcp.status.validation_passed_format", response.serverCount)
                } else {
                    self.error = response.errors.joined(separator: "\n")
                }
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    private func saveConfig() {
        guard !self.rawJson.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            self.error = L10n.tr("settings.mcp.error.empty_json")
            return
        }

        self.isSaving = true
        self.status = nil
        self.error = nil
        let capturedJson = self.rawJson
        let request = MCPConfigMutationRequest(
            rawJson: capturedJson,
            secretKeys: self.extractSecretKeyMap(from: capturedJson)
        )

        Task {
            defer { self.isSaving = false }
            do {
                let response = try await DaemonClient.shared.saveMCPConfig(request)
                self.rawJson = response.rawJson
                self.servers = response.servers
                self.status = response.message
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    private func resetConfig() {
        self.status = nil
        self.error = nil
        self.isSaving = true

        Task {
            defer { self.isSaving = false }
            do {
                let response = try await DaemonClient.shared.resetMCPConfig()
                self.rawJson = response.rawJson
                self.servers = []
                self.status = response.message
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    private func extractSecretKeyMap(from json: String) -> [String: [String]]? {
        guard let data = json.data(using: .utf8),
              let doc = try? JSONDecoder().decode(InputDoc.self, from: data)
        else {
            return self.lastValidSecretMap
        }

        let allServers = doc.mcpServers ?? doc.servers ?? [:]
        var secretMap: [String: [String]] = [:]

        for (serverName, entry) in allServers {
            guard let env = entry.env else {
                continue
            }

            let keys = env.compactMap { key, value -> String? in
                let upper = key.uppercased()
                if value.isEmpty || value.contains("${") || value.hasPrefix("bobe-secret://") {
                    return nil
                }
                if upper.contains("SECRET")
                    || upper.contains("TOKEN")
                    || upper.contains("PASSWORD")
                    || upper.hasSuffix("API_KEY") {
                    return key
                }
                return nil
            }

            if !keys.isEmpty {
                secretMap[serverName] = keys.sorted()
            }
        }

        let result = secretMap.isEmpty ? nil : secretMap
        self.lastValidSecretMap = result
        return result
    }
}

private struct InputServerEntry: Decodable {
    let env: [String: String]?
}

private struct InputDoc: Decodable {
    let mcpServers: [String: InputServerEntry]?
    let servers: [String: InputServerEntry]?
}
