import SwiftUI

struct LocalRuntimeModels: Equatable, Sendable {
    static let setup = Self(
        profileID: "local",
        chatModel: "qwen2.5:7b-instruct",
        batchModel: "qwen2.5:7b-instruct"
    )

    let profileID: String
    let chatModel: String
    let batchModel: String

    var installRequest: LocalRuntimeInstallRequest {
        LocalRuntimeInstallRequest(
            profile: self.profileID,
            chatModel: self.chatModel,
            batchModel: self.batchModel
        )
    }

    var activationRequest: ProviderActivationRequest {
        .ollama(chatModel: self.chatModel, batchModel: self.batchModel)
    }
}

enum LocalRuntimeInstallError: LocalizedError {
    case cancelFailed

    var errorDescription: String? {
        L10n.tr("setup.local.cancel_failed")
    }
}

@MainActor
enum LocalRuntimeInstall {
    static func cancel() async throws {
        try await self.cancel { try await DaemonClient.shared.cancelLocalRuntimeInstall() }
    }

    static func cancel(using request: () async throws -> Void) async throws {
        do {
            try await request()
        } catch {
            throw LocalRuntimeInstallError.cancelFailed
        }
    }
}

struct LocalSetupStepView: View {
    let onContinue: () -> Void
    var onCriticalOperationChange: (Bool) -> Void = { _ in }

    @State private var snapshot: LocalRuntimeSnapshot?
    @State private var ramGB: Int? = systemMemoryGB()
    @State private var streamTask: Task<Void, Never>?
    @State private var startError: String?
    @State private var cancelError: String?
    @State private var isCanceling = false
    @Environment(\.theme) private var theme

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            SetupStepHeader(title: L10n.tr("setup.local.title"), message: L10n.tr("setup.local.body"))
                .padding(.top, 8)

            self.hardwareCard

            VStack(spacing: 12) {
                self.downloadRow(
                    title: L10n.tr("setup.local.runtime"),
                    percent: self.snapshot.map { $0.runtime.percent ?? 0 }
                )
                self.downloadRow(
                    title: L10n.tr("setup.local.chat_model"),
                    percent: self.snapshot.map { $0.chatModel.percent ?? 0 }
                )
            }

            if let error = self.startError {
                Text(error)
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.error)
            }

            Spacer()

            self.actionRow
                .accessibilityIdentifier("setup.local.action")
        }
        .task {
            await self.startInstall()
        }
        .onDisappear {
            self.streamTask?.cancel()
        }
    }

    @ViewBuilder
    private var hardwareCard: some View {
        if DaemonConfig.endpoint.managesLocalProcess, let ramGB = self.ramGB, ramGB < 16 {
            HStack(spacing: 10) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(self.theme.colors.warning)
                Text(L10n.tr("setup.local.ram_warning"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
            }
            .padding(10)
            .background(
                RoundedRectangle(cornerRadius: 8)
                    .fill(self.theme.colors.warning.opacity(0.1))
            )
        }
    }

    private func downloadRow(title: String, percent: Int?) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack {
                Text(title)
                    .bobeTextStyle(.setupHeading)
                Spacer()
                Text(percent.map { "\($0)%" } ?? "—")
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
            }
            BobeLinearProgressBar(progress: Double(percent ?? 0) / 100.0, tint: self.theme.colors.secondary)
        }
    }

    @ViewBuilder
    private var actionRow: some View {
        let status = self.snapshot?.status ?? .running
        switch status {
        case .complete:
            Button(L10n.tr("setup.welcome.continue"), action: self.onContinue)
                .bobeButton(.primary, size: .regular)
                .keyboardShortcut(.defaultAction)
        case .canceled:
            Button(L10n.tr("setup.local.retry")) {
                Task { await self.startInstall() }
            }
            .bobeButton(.primary, size: .regular)
        case .failed:
            VStack(spacing: 8) {
                if let error = self.snapshot?.error {
                    Text(error)
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.error)
                        .multilineTextAlignment(.center)
                }
                Button(L10n.tr("setup.local.retry")) {
                    Task { await self.startInstall() }
                }
                .bobeButton(.primary, size: .regular)
            }
        case .idle, .running:
            VStack(spacing: 8) {
                if let cancelError = self.cancelError {
                    Text(cancelError)
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.error)
                        .multilineTextAlignment(.center)
                }
                Button(action: self.cancelInstall) {
                    HStack(spacing: 6) {
                        if self.isCanceling {
                            BobeSpinner(size: 12)
                        }
                        Text(L10n.tr(self.isCanceling ? "setup.local.canceling" : "setup.local.cancel"))
                    }
                }
                .bobeButton(.secondary, size: .small)
                .disabled(self.isCanceling)
            }
        }
    }

    private func startInstall() async {
        self.startError = nil
        self.cancelError = nil
        do {
            try await DaemonClient.shared.startLocalRuntimeInstall(LocalRuntimeModels.setup.installRequest)
        } catch {
            if !error.localizedDescription.lowercased().contains("conflict") {
                self.startError = error.localizedDescription
            }
        }
        self.subscribeToStatus()
    }

    private func subscribeToStatus() {
        self.streamTask?.cancel()
        self.streamTask = Task { @MainActor in
            do {
                try await DaemonClient.shared.streamLocalRuntimeStatus { snap in
                    Task { @MainActor in
                        self.snapshot = snap
                        self.startError = nil
                    }
                }
            } catch {
                self.startError = error.localizedDescription
            }
        }
    }

    private func cancelInstall() {
        guard !self.isCanceling else { return }
        self.isCanceling = true
        self.cancelError = nil
        self.onCriticalOperationChange(true)
        Task { @MainActor in
            do {
                try await LocalRuntimeInstall.cancel()
            } catch {
                self.cancelError = error.localizedDescription
            }
            self.isCanceling = false
            self.onCriticalOperationChange(false)
            self.subscribeToStatus()
        }
    }
}

private func systemMemoryGB() -> Int? {
    var memory: UInt64 = 0
    var size = MemoryLayout<UInt64>.size
    let result = sysctlbyname("hw.memsize", &memory, &size, nil, 0)
    guard result == 0 else { return nil }
    return Int(memory / (1024 * 1024 * 1024))
}
