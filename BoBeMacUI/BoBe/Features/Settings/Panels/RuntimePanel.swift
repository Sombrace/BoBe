import SwiftUI

struct RuntimePanel: View {
    @Environment(\.theme) private var theme
    @State private var hello: HelloResponse?
    @State private var readiness: HelloReadiness?
    @State private var runtimeIssue: RuntimeIssue?
    @State private var errorMessage: String?
    @State private var cliStatus: String?
    @State private var isWorking = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                self.runtimeCard
                self.actionsCard
                if let runtimeIssue {
                    VStack(alignment: .leading, spacing: 8) {
                        Label(runtimeIssue.message, systemImage: "exclamationmark.triangle.fill")
                            .bobeTextStyle(.helper)
                            .foregroundStyle(self.theme.colors.error)
                        Button(L10n.tr("settings.runtime.repair")) {
                            SetupWindowManager.shared.showRuntimeRepair(issue: runtimeIssue)
                        }
                        .bobeButton(.secondary)
                    }
                } else if let errorMessage {
                    Label(errorMessage, systemImage: "exclamationmark.triangle.fill")
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.error)
                }
            }
            .padding(24)
        }
        .task { await self.refresh() }
    }

    private var runtimeCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(EndpointStore.shared.profile?.friendlyName ?? "BoBe", systemImage: "server.rack")
                .bobeTextStyle(.setupHeading)
                .foregroundStyle(self.theme.colors.text)
            self.detailRow(L10n.tr("settings.runtime.mode"), value: self.modeLabel)
            self.detailRow(
                L10n.tr("settings.runtime.address"),
                value: EndpointStore.shared.profile?.baseURL.absoluteString ?? DaemonConfig.baseURL
            )
            if let hello {
                self.detailRow(L10n.tr("settings.runtime.version"), value: hello.daemonVersion)
                self.detailRow(
                    L10n.tr("settings.runtime.authority"),
                    value: hello.runtimeAuthorityId.uuidString
                )
            }
            if let readiness {
                self.detailRow(
                    L10n.tr("settings.runtime.status"),
                    value: self.statusDescription(readiness.state)
                )
                if readiness.state != .ready {
                    ForEach(readiness.blockingReasons, id: \.self) { reason in
                        Label(self.blockingReasonDescription(reason), systemImage: "info.circle")
                            .bobeTextStyle(.helper)
                            .foregroundStyle(self.theme.colors.textMuted)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        }
        .padding(16)
        .background(self.cardBackground)
    }

    private var actionsCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(L10n.tr("settings.runtime.actions"))
                .bobeTextStyle(.setupHeading)
                .foregroundStyle(self.theme.colors.text)
            HStack {
                Button(L10n.tr("settings.runtime.check")) {
                    Task { await self.refresh() }
                }
                .bobeButton(.secondary)
                .disabled(self.isWorking)
                Button(L10n.tr("settings.runtime.switch")) {
                    Task { @MainActor in
                        await self.beginRuntimeSwitch()
                    }
                }
                .bobeButton(.secondary)
                .disabled(self.isWorking)
                if DaemonConfig.endpoint.canRestartRuntime {
                    Button(L10n.tr("overlay.error.daemon_restart")) {
                        Task { await self.restartRuntime() }
                    }
                    .bobeButton(.secondary)
                    .disabled(self.isWorking)
                }
            }
            BobeHairline()
            Button(L10n.tr("settings.runtime.cli.install")) {
                do {
                    let path = try CommandLineToolInstaller.install()
                    self.cliStatus = L10n.tr("settings.runtime.cli.installed_format", path.path)
                } catch {
                    self.cliStatus = error.localizedDescription
                }
            }
            .bobeButton(.primary)
            if let cliStatus {
                Text(cliStatus)
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .textSelection(.enabled)
            }
        }
        .padding(16)
        .background(self.cardBackground)
    }

    private var cardBackground: some View {
        Color.clear.bobeRaised(cornerRadius: BobeRadius.card)
    }

    private func detailRow(_ label: String, value: String) -> some View {
        HStack(alignment: .firstTextBaseline) {
            Text(label)
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.textMuted)
                .frame(width: 92, alignment: .leading)
            Text(value)
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.text)
                .textSelection(.enabled)
            Spacer()
        }
    }

    private func statusDescription(_ state: ReadinessStateWire) -> String {
        switch state {
        case .ready: L10n.tr("settings.runtime.status.ready")
        case .initializing: L10n.tr("settings.runtime.status.initializing")
        case .notReady, .unknown: L10n.tr("settings.runtime.status.not_ready")
        }
    }

    private func blockingReasonDescription(_ reason: String) -> String {
        switch reason {
        case "initialization_incomplete":
            L10n.tr("settings.runtime.blocking.initialization_incomplete")
        case "database_unavailable":
            L10n.tr("settings.runtime.blocking.database_unavailable")
        default:
            L10n.tr("settings.runtime.blocking.unknown")
        }
    }

    private var modeLabel: String {
        switch EndpointStore.shared.profile?.mode {
        case .managedLocal: L10n.tr("settings.runtime.mode.managed")
        case .attachedLocal: L10n.tr("settings.runtime.mode.attached")
        case .remote: L10n.tr("settings.runtime.mode.remote")
        case nil: L10n.tr("settings.common.unknown")
        }
    }

    @MainActor
    private func beginRuntimeSwitch() async {
        guard !SetupWindowManager.shared.isPresented else {
            SetupWindowManager.shared.show()
            return
        }
        let previousProfile = EndpointStore.shared.profile
        let previousCredential = previousProfile.flatMap(EndpointStore.shared.credential)
        self.isWorking = true
        defer { self.isWorking = false }
        await SettingsStore.shared.flushPendingSave()
        BobeStore.shared.disconnect()
        VoicePipeline.shared.disconnect()
        OverlayWindowManager.shared.hide()
        SettingsWindowManager.shared.close()
        if previousProfile?.mode == .managedLocal {
            await BackendService.shared.stop()
        }
        let shown = SetupWindowManager.shared.show(
            initialStep: .runtimeLocation,
            onCancel: {
                Task { @MainActor in
                    let candidateProfile = EndpointStore.shared.profile
                    if candidateProfile?.mode == .managedLocal,
                       previousProfile?.mode != .managedLocal {
                        await BackendService.shared.stop()
                    }
                    do {
                        if let previousProfile {
                            try EndpointStore.shared.persist(
                                previousProfile,
                                credential: previousCredential,
                                deletePreviousCredential: false
                            )
                        } else {
                            EndpointStore.shared.clear()
                        }
                        if candidateProfile?.credentialReference != previousProfile?.credentialReference {
                            EndpointStore.shared.deleteCredential(
                                reference: candidateProfile?.credentialReference
                            )
                        }
                        if previousProfile?.mode == .managedLocal {
                            try await BackendService.shared.start()
                        }
                    } catch {
                        self.errorMessage = error.localizedDescription
                    }
                    await VoicePipeline.shared.refreshDaemonState()
                    BobeStore.shared.connect()
                    OverlayWindowManager.shared.show()
                }
            },
            onComplete: {
                let committedReference = EndpointStore.shared.profile?.credentialReference
                if previousProfile?.credentialReference != committedReference {
                    EndpointStore.shared.deleteCredential(
                        reference: previousProfile?.credentialReference
                    )
                }
            }
        )
        if !shown {
            if previousProfile?.mode == .managedLocal {
                try? await BackendService.shared.start()
            }
            await VoicePipeline.shared.refreshDaemonState()
            BobeStore.shared.connect()
            OverlayWindowManager.shared.show()
        }
    }

    @MainActor
    private func restartRuntime() async {
        guard !self.isWorking else { return }
        self.isWorking = true
        defer { self.isWorking = false }
        self.hello = nil
        self.readiness = nil
        self.runtimeIssue = nil
        self.errorMessage = nil
        do {
            try await BackendService.shared.userRestart()
            await self.refreshAuthenticatedConnection()
        } catch {
            self.runtimeIssue = RuntimeIssue(error)
        }
    }

    @MainActor
    private func refresh() async {
        guard !self.isWorking else { return }
        self.isWorking = true
        defer { self.isWorking = false }
        await self.refreshAuthenticatedConnection()
    }

    @MainActor
    private func refreshAuthenticatedConnection() async {
        do {
            guard let attach = try await EndpointAttachService.shared.attachConfiguredEndpoint() else {
                self.hello = nil
                self.readiness = nil
                self.runtimeIssue = RuntimeIssue(AttachError.unauthorized)
                return
            }
            self.hello = attach.hello
            self.readiness = attach.hello.readiness
            self.runtimeIssue = nil
            self.errorMessage = nil
        } catch {
            self.hello = nil
            if case let AttachError.notReady(reasons) = error {
                self.readiness = HelloReadiness(state: .notReady, blockingReasons: reasons)
            } else {
                self.readiness = nil
            }
            self.runtimeIssue = RuntimeIssue(error)
            self.errorMessage = nil
        }
    }
}
