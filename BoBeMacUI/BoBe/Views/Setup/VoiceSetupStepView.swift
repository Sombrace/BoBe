import SwiftUI

struct VoiceSetupStepView: View {
    let onContinue: () -> Void

    @Environment(\.theme) private var theme
    @State private var installer = VoiceModelInstaller.shared
    @State private var phase: Phase = .checking
    @State private var errorMessage: String?
    @State private var installTask: Task<Void, Never>?
    @State private var installGeneration: UInt64 = 0
    @State private var cancelError: String?

    enum Phase {
        case checking, idle, installing, canceling, complete, failed
    }

    private var showsVoiceModel: Bool {
        VoiceTtsPreference.shared.backend == .serverPocket
            && self.installer.status(of: .pocketTts) != .unknown
    }

    private var hearingLabel: String {
        switch self.installer.selectedSttStatus {
        case .unknown, .missing:
            return L10n.tr("setup.voice.stt_label.waiting")
        case let .downloading(percent, _, _):
            return String(
                format: L10n.tr("setup.voice.stt_label.progress_format"),
                percent ?? 0
            )
        case .installed:
            return L10n.tr("setup.voice.stt_label.ready")
        case let .failed(message):
            return String(format: L10n.tr("setup.voice.stt_label.failed_format"), message)
        }
    }

    var body: some View {
        VStack(spacing: 18) {
            SetupStepHeader(
                title: L10n.tr("setup.voice.title"),
                message: L10n.tr("setup.voice.body")
            )
            .padding(.top, 8)

            self.bodyContent

            Spacer()

            self.actionRow
        }
        .task { await self.refreshStatus() }
        .onDisappear {
            self.installGeneration &+= 1
            self.installTask?.cancel()
            self.installTask = nil
        }
    }

    @ViewBuilder
    private var bodyContent: some View {
        switch self.phase {
        case .checking:
            BobeSpinner(size: 18)
                .padding(.top, 20)
        case .idle:
            self.installPrompt
        case .installing:
            self.progressList
        case .canceling:
            HStack(spacing: 10) {
                BobeSpinner(size: 14)
                Text(L10n.tr("setup.voice.canceling"))
                    .bobeTextStyle(.setupBody)
                    .foregroundStyle(self.theme.colors.textMuted)
            }
            .padding(.top, 20)
        case .complete:
            self.completeSummary
        case .failed:
            self.failureSummary
        }
    }

    private var installPrompt: some View {
        VStack(spacing: 10) {
            if self.showsVoiceModel {
                VoiceModelRow(role: .voice, status: self.installer.status(of: .pocketTts))
                    .setupStagger(0)
            }
            VoiceModelRow(role: .hearing, status: self.installer.selectedSttStatus)
                .setupStagger(self.showsVoiceModel ? 1 : 0)
        }
    }

    private var progressList: some View {
        VStack(spacing: 12) {
            if self.showsVoiceModel {
                VoiceModelRow(role: .voice, status: self.installer.status(of: .pocketTts))
            }
            HStack(spacing: 10) {
                switch self.installer.selectedSttStatus {
                case .installed:
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(self.theme.colors.secondary)
                case .unknown, .missing, .downloading:
                    BobeSpinner(size: 14)
                case .failed:
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(self.theme.colors.primary)
                }
                Text(self.hearingLabel)
                    .bobeTextStyle(.setupBody)
                    .foregroundStyle(self.theme.colors.textMuted)
                Spacer()
            }
            .padding(.horizontal, 4)
            if let cancelError = self.cancelError {
                Text(cancelError)
                    .bobeTextStyle(.setupBody)
                    .foregroundStyle(self.theme.colors.primary)
                    .multilineTextAlignment(.center)
            }
        }
        .task { await self.awaitInstall() }
    }

    private var completeSummary: some View {
        HStack(spacing: 10) {
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(self.theme.colors.secondary)
            Text(L10n.tr("setup.voice.complete"))
                .bobeTextStyle(.setupBody)
                .foregroundStyle(self.theme.colors.text)
        }
        .padding(.vertical, 16)
    }

    private var failureSummary: some View {
        VStack(spacing: 8) {
            HStack(spacing: 10) {
                Image(systemName: "exclamationmark.triangle.fill")
                    .foregroundStyle(self.theme.colors.primary)
                Text(L10n.tr("setup.voice.failed"))
                    .bobeTextStyle(.setupBody)
                    .foregroundStyle(self.theme.colors.text)
            }
            if let errorMessage = self.errorMessage {
                Text(errorMessage)
                    .bobeTextStyle(.setupBody)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .multilineTextAlignment(.center)
            }
        }
    }

    @ViewBuilder
    private var actionRow: some View {
        switch self.phase {
        case .checking:
            EmptyView()
        case .idle:
            HStack(spacing: 12) {
                Button(L10n.tr("setup.voice.skip")) {
                    VoiceSetupPreference.setDeferred(true)
                    self.onContinue()
                }
                    .bobeButton(.secondary, size: .regular)
                Button(L10n.tr("setup.voice.install_button")) {
                    self.installTask = Task { await self.kickoff() }
                }
                    .bobeButton(.primary, size: .regular)
                    .keyboardShortcut(.defaultAction)
            }
        case .installing:
            Button(L10n.tr("setup.voice.cancel")) {
                Task { await self.cancelInstall() }
            }
            .bobeButton(.secondary, size: .regular)
        case .canceling:
            Button(L10n.tr("setup.voice.canceling")) {}
                .bobeButton(.secondary, size: .regular)
                .disabled(true)
        case .complete:
            Button(L10n.tr("setup.voice.continue"), action: self.onContinue)
                .bobeButton(.primary, size: .regular)
                .keyboardShortcut(.defaultAction)
        case .failed:
            HStack(spacing: 12) {
                Button(L10n.tr("setup.voice.continue_without"), action: self.continueWithoutVoice)
                    .bobeButton(.secondary, size: .regular)
                Button(L10n.tr("setup.voice.try_again")) {
                    self.installTask = Task { await self.kickoff() }
                }
                .bobeButton(.primary, size: .regular)
                .keyboardShortcut(.defaultAction)
            }
        }
    }

    private func continueWithoutVoice() {
        VoiceSetupPreference.setDeferred(true)
        self.onContinue()
    }

    private func cancelInstall() async {
        guard self.phase == .installing else { return }
        self.installGeneration &+= 1
        self.installTask?.cancel()
        self.installTask = nil
        self.cancelError = nil
        self.phase = .canceling
        do {
            try await self.installer.cancel()
            await self.refreshStatus()
        } catch {
            self.cancelError = L10n.tr("setup.voice.cancel_failed_format", error.localizedDescription)
            self.phase = .installing
        }
    }

    private func refreshStatus() async {
        self.errorMessage = nil
        self.phase = .checking
        let generation = self.installGeneration
        await self.installer.refresh()
        guard !Task.isCancelled, generation == self.installGeneration else { return }
        self.show(self.installer.outcome)
        if self.phase == .installing { self.installer.resume() }
    }

    private func kickoff() async {
        guard self.phase != .installing, self.phase != .canceling else { return }
        self.installGeneration &+= 1
        let generation = self.installGeneration
        self.cancelError = nil
        self.phase = .installing
        do {
            try await self.installer.installSelected()
        } catch {
            guard !Task.isCancelled, generation == self.installGeneration else { return }
            self.errorMessage = error.localizedDescription
            self.phase = .failed
        }
    }

    private func awaitInstall() async {
        guard let outcome = await self.installer.settled(),
              !Task.isCancelled,
              self.phase == .installing
        else { return }
        self.show(outcome)
    }

    private func show(_ outcome: VoiceInstallOutcome) {
        switch outcome {
        case .installing:
            self.phase = .installing
        case .ready:
            self.phase = .complete
        case .stopped:
            self.phase = .idle
        case let .failed(message):
            self.errorMessage = message
            self.phase = .failed
        }
    }
}
