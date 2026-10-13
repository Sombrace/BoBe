import SwiftUI

struct VoiceSetupStepView: View {
    let onContinue: () -> Void

    @Environment(\.theme) private var theme
    @State private var pipeline = VoicePipeline.shared
    @State private var phase: Phase = .checking
    @State private var errorMessage: String?
    @State private var installTask: Task<Void, Never>?
    @State private var installGeneration: UInt64 = 0
    @State private var preparingModels = false
    @State private var cancelingInstall = false

    enum Phase {
        case checking, idle, installing, complete, failed
    }

    private var status: VoiceInstallSnapshot? {
        self.pipeline.installSnapshot
    }

    private var allReady: Bool {
        self.pipeline.modelPreparation == .ready
    }

    private var selectedTtsBackend: VoiceTtsBackend {
        VoiceTtsPreference.shared.backend
    }

    private var visibleDaemonModels: [VoiceModelProgress] {
        guard self.selectedTtsBackend == .serverPocket else { return [] }
        return (self.status?.models ?? []).filter { $0.kind == VoiceWire.modelKindTts }
    }

    private var sttLabel: String {
        switch self.pipeline.sttStatus {
        case .notLoaded: return L10n.tr("setup.voice.stt_label.waiting")
        case .downloading:
            let percent = self.pipeline.sttLoadProgress?.percent ?? 0
            return String(
                format: L10n.tr("setup.voice.stt_label.progress_format"),
                percent
            )
        case .ready: return L10n.tr("setup.voice.stt_label.ready")
        case let .failed(msg): return String(format: L10n.tr("setup.voice.stt_label.failed_format"), msg)
        }
    }

    private var sttModel: VoiceModelProgress {
        let status = switch self.pipeline.sttStatus {
        case .ready: "already installed"
        case .downloading: "downloading"
        case .failed: "failed"
        case .notLoaded: "pending"
        }
        return VoiceModelProgress(
            kind: "stt",
            label: L10n.tr("setup.voice.model.stt"),
            status: status,
            bytesDownloaded: 0,
            bytesTotal: nil,
            percent: self.pipeline.sttLoadProgress?.percent
        )
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
                .disabled(self.cancelingInstall)
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
        case .complete:
            self.completeSummary
        case .failed:
            self.failureSummary
        }
    }

    private var installPrompt: some View {
        VStack(spacing: 10) {
            ForEach(Array(self.visibleDaemonModels.enumerated()), id: \.element.id) { index, model in
                VoiceModelRow(model: model)
                    .setupStagger(index)
            }
            VoiceModelRow(model: self.sttModel)
                .setupStagger(self.visibleDaemonModels.count)
        }
    }

    private var progressList: some View {
        VStack(spacing: 12) {
            ForEach(self.visibleDaemonModels, id: \.id) { model in
                VoiceModelRow(model: model)
            }
            HStack(spacing: 10) {
                switch self.pipeline.sttStatus {
                case .ready:
                    Image(systemName: "checkmark.circle.fill")
                        .foregroundStyle(self.theme.colors.secondary)
                case .downloading, .notLoaded:
                    BobeSpinner(size: 14)
                case .failed:
                    Image(systemName: "exclamationmark.triangle.fill")
                        .foregroundStyle(self.theme.colors.primary)
                }
                Text(self.sttLabel)
                    .bobeTextStyle(.setupBody)
                    .foregroundStyle(self.theme.colors.textMuted)
                Spacer()
            }
            .padding(.horizontal, 4)
        }
        .task { await self.poll() }
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
                self.installGeneration &+= 1
                self.installTask?.cancel()
                self.installTask = nil
                self.preparingModels = false
                self.phase = .idle
                self.cancelingInstall = true
                Task {
                    await self.cancelAll()
                    self.cancelingInstall = false
                }
            }
            .bobeButton(.secondary, size: .regular)
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

    private func refreshStatus() async {
        self.errorMessage = nil
        self.phase = .checking
        let generation = self.installGeneration
        await self.pipeline.refreshDaemonState()
        guard !Task.isCancelled, generation == self.installGeneration else { return }
        if self.pipeline.sttStatus == .notLoaded { self.pipeline.bootstrapSttPresence() }
        guard let s = self.status else {
            self.errorMessage = L10n.tr("setup.voice.daemon_unreachable")
            self.phase = .failed
            return
        }
        if self.allReady {
            self.phase = .complete
        } else if s.isRunning || self.pipeline.sttStatus == .downloading {
            self.phase = .installing
            Task { await self.pipeline.ensureSttLoaded() }
        } else if case let .failed(message) = self.pipeline.modelPreparation {
            self.errorMessage = message
            self.phase = .failed
        } else {
            self.phase = .idle
        }
    }

    private func kickoff() async {
        guard self.phase != .installing else { return }
        self.installGeneration &+= 1
        let generation = self.installGeneration
        self.preparingModels = true
        defer { if generation == self.installGeneration { self.preparingModels = false } }
        self.phase = .installing
        VoiceSetupPreference.setDeferred(false)
        async let stt: Void = self.pipeline.ensureSttLoaded()
        do {
            try await self.pipeline.prepareSelectedTts()
            await stt
            guard !Task.isCancelled, generation == self.installGeneration else { return }
            await self.refreshStatus()
        } catch {
            guard !Task.isCancelled, generation == self.installGeneration else { return }
            self.errorMessage = error.localizedDescription
            self.phase = .failed
        }
    }

    private func cancelAll() async {
        let client = DaemonClient(endpoint: self.pipeline.daemonStateSelection.endpoint)
        async let daemon: Void = {
            try? await client.cancelVoiceInstall()
        }()
        async let stt: Void = self.pipeline.cancelSttLoading()
        _ = await (daemon, stt)
    }

    private func poll() async {
        while !Task.isCancelled, self.phase == .installing {
            try? await Task.sleep(for: .milliseconds(500))
            guard !Task.isCancelled, self.phase == .installing else { return }
            if self.preparingModels { continue }
            if case let .failed(msg) = self.pipeline.sttStatus {
                self.errorMessage = msg
                await self.cancelAll()
                guard !Task.isCancelled, self.phase == .installing else { return }
                self.phase = .failed
                return
            }
            await self.pipeline.refreshDaemonState()
            guard !Task.isCancelled, self.phase == .installing else { return }
            if self.preparingModels { continue }
            guard let s = self.status else {
                self.errorMessage = L10n.tr("setup.voice.daemon_unreachable")
                self.phase = .failed
                return
            }
            if self.allReady {
                self.phase = .complete
                return
            }
            switch s.status {
            case .failed:
                self.errorMessage = s.error ?? L10n.tr("setup.voice.poll_failure")
                self.phase = .failed; return
            case .canceled:
                self.phase = .idle; return
            case .idle, .complete:
                if self.pipeline.modelPreparation == .missing {
                    self.phase = .idle
                    return
                }
            case .running:
                continue
            }
        }
    }
}
