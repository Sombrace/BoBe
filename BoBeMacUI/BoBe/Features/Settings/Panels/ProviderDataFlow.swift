import SwiftUI

struct ProviderDataFlow: View {
    @Environment(\.theme) private var theme
    @State private var snapshot: ProvidersResponse?
    @State private var selection = EndpointStore.shared.selection
    @State private var error: String?

    private var hostName: String {
        self.selection.endpoint.managesLocalProcess
            ? L10n.tr("settings.providers.this_mac")
            : EndpointStore.shared.profile?.friendlyName ?? L10n.tr("settings.providers.host")
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            HStack(spacing: 10) {
                Image(systemName: "internaldrive")
                    .foregroundStyle(self.theme.colors.primary)
                Text(L10n.tr("settings.providers.storage")).bobeTextStyle(.settingsBody)
                Spacer()
                Label(self.hostName, systemImage: self.selection.endpoint.managesLocalProcess ? "desktopcomputer" : "server.rack")
                    .bobeTextStyle(.settingsBody)
                    .padding(.horizontal, 10)
                    .padding(.vertical, 6)
                    .background(Capsule().fill(self.theme.colors.border.opacity(0.25)))
                    .help(L10n.tr("settings.providers.storage.help"))
            }
            if let snapshot {
                Grid(horizontalSpacing: 12, verticalSpacing: 12) {
                    ForEach(ProviderWorkload.allCases) { workload in
                        let route = snapshot.activeRoutes.route(for: workload)
                        let profile = snapshot.activeProfiles.first { $0.id == route.profile }
                        GridRow {
                            Label(workload.label, systemImage: workload.symbol)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            Image(systemName: "arrow.right")
                                .foregroundStyle(self.theme.colors.textMuted)
                                .accessibilityHidden(true)
                            if route.model == nil {
                                Text(L10n.tr("settings.providers.off"))
                                    .foregroundStyle(self.theme.colors.textMuted)
                                    .frame(maxWidth: .infinity, alignment: .trailing)
                            } else if let profile {
                                Label(
                                    profile.runsOnAuthority ? self.hostName : profile.destination,
                                    systemImage: profile.runsOnAuthority ? "server.rack" : "network"
                                )
                                .lineLimit(1).truncationMode(.middle)
                                .frame(maxWidth: .infinity, alignment: .trailing)
                                .help(route.model ?? "")
                            } else {
                                Image(systemName: "questionmark.circle")
                                    .accessibilityLabel(L10n.tr("settings.providers.unknown"))
                            }
                        }
                        .bobeTextStyle(.settingsBody)
                        .accessibilityElement(children: .combine)
                    }
                }
            } else if self.error == nil {
                BobeSpinner(size: 16).frame(maxWidth: .infinity)
            }
            HStack(spacing: 6) {
                Image(systemName: "network")
                Text(L10n.tr("settings.providers.tools_network"))
            }
            .bobeTextStyle(.helper)
            .foregroundStyle(self.theme.colors.textMuted)
            .help(L10n.tr("settings.agent.tools.description"))
            if let error {
                SettingsErrorBanner(message: error)
                Button(L10n.tr("setup.cloud_auth.retry")) { Task { await self.load() } }
                    .bobeButton(.secondary, size: .small)
            }
        }
        .task(id: self.selection) { await self.load() }
        .onReceive(NotificationCenter.default.publisher(for: .bobeEndpointIdentityChanged)) { _ in
            self.selection = EndpointStore.shared.selection
            self.snapshot = nil
            self.error = nil
        }
    }

    @MainActor
    private func load() async {
        let selection = self.selection
        do {
            let response = try await DaemonClient(endpoint: selection.endpoint).listProviders()
            guard !Task.isCancelled, EndpointStore.shared.selection == selection else { return }
            self.snapshot = response
            self.error = nil
        } catch {
            guard !Task.isCancelled, EndpointStore.shared.selection == selection else { return }
            self.error = error.localizedDescription
        }
    }
}
