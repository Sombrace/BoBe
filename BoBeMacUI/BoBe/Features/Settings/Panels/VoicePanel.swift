import SwiftUI

private enum VoiceInstallTarget {
    case daemonTts
    case clientStt(language: String)
}

private enum VoiceModelPaths {
    static var kokoro: String {
        DaemonConfig.managedDataDirectory
            .appendingPathComponent("models", isDirectory: true)
            .appendingPathComponent(VoiceWire.kokoroModelDir, isDirectory: true)
            .path
    }
    static let parakeet = "~/Library/Application Support/FluidAudio/Models/parakeet-eou-streaming/"
    static let nemotron = "~/Library/Application Support/FluidAudio/Models/nemotron-multilingual/"
}

struct VoicePanel: View {
    @Environment(VoicePipeline.self) private var pipeline
    @Environment(SettingsStore.self) private var store
    @Environment(ExpertMode.self) private var expertMode
    @State private var ttsPreference = VoiceTtsPreference.shared
    @State private var sttPreference = VoiceSttPreferenceStore.shared
    @State private var remoteSpeechConsent = RemoteSpeechConsent.shared
    @State private var isReinstalling = false
    @State private var installError: String?
    @State private var clientTtsInstalled = FluidAudioSupertonicModelPresence.isInstalled()
    @State private var statusPollTask: Task<Void, Never>?
    @Environment(\.theme) private var theme

    private var installStatus: VoiceInstallSnapshot? {
        self.pipeline.installSnapshot
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text(L10n.tr("settings.voice.description"))
                    .bobeTextStyle(.settingsBody)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .fixedSize(horizontal: false, vertical: true)

                if let error = self.store.error ?? self.installError {
                    SettingsErrorBanner(message: error)
                }
                if let savedMessage = self.store.savedMessage {
                    SettingsSavedToast(message: savedMessage)
                }

                if self.store.settings != nil {
                    self.engineSection
                    self.modelsSection
                } else if self.store.isLoading {
                    HStack(spacing: 8) {
                        BobeSpinner(size: 14)
                        Text(L10n.tr("settings.voice.loading"))
                            .bobeTextStyle(.settingsBody)
                            .foregroundStyle(self.theme.colors.textMuted)
                    }
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 40)
                } else {
                    HStack(spacing: 8) {
                        Image(systemName: "exclamationmark.circle")
                            .foregroundStyle(self.theme.colors.textMuted)
                        Text(L10n.tr("settings.voice.empty.unreachable"))
                            .bobeTextStyle(.settingsBody)
                            .foregroundStyle(self.theme.colors.textMuted)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .frame(maxWidth: .infinity, alignment: .center)
                    .padding(.top, 40)
                }
            }
            .padding(24)
        }
        .task {
            await self.loadAll()
        }
        .onDisappear {
            self.statusPollTask?.cancel()
            self.store.cancelToast()
        }
    }

    private var engineSection: some View {
        CollapsibleSection(
            title: L10n.tr("settings.voice.section.preferences"),
            icon: "waveform",
            description: nil
        ) {
            VStack(alignment: .leading, spacing: 12) {
                SettingsRow(
                    label: L10n.tr("settings.voice.enabled"),
                    description: L10n.tr("settings.voice.enabled.description")
                ) {
                    BobeToggle(
                        isOn: self.store.binding(\.voiceEnabled, fallback: true),
                        accessibilityLabel: L10n.tr("settings.voice.enabled")
                    )
                }

                SettingsRow(
                    label: L10n.tr("settings.voice.language"),
                    description: L10n.tr("settings.voice.language.description")
                ) {
                    BobeMenuPicker(
                        selection: self.store.binding(\.voiceSttLanguage, fallback: "en"),
                        options: VoiceLanguages.all,
                        label: VoiceLanguages.displayName(for:),
                        width: 240
                    )
                }

                if let advice = self.languageAdvice {
                    VoiceLanguageAdviceStrip(message: self.adviceMessage(advice)) {
                        self.apply(advice)
                    }
                    .transition(.bobeRise)
                }

                SettingsRow(
                    label: L10n.tr("settings.voice.pause_sensitivity"),
                    description: L10n.tr("settings.voice.pause_sensitivity.description")
                ) {
                    BobeMenuPicker(
                        selection: self.store.binding(\.voicePauseSensitivity, fallback: "balanced"),
                        options: ["tight", "balanced", "patient"],
                        label: { L10n.tr("settings.voice.pause_sensitivity.\($0)") },
                        width: 240
                    )
                }

                SettingsRow(
                    label: L10n.tr("settings.voice.persona"),
                    description: L10n.tr("settings.voice.persona.description")
                ) {
                    BobeMenuPicker(
                        selection: self.store.binding(\.voicePersona, fallback: VoiceWire.defaultPersona),
                        options: KokoroVoices.allSlots,
                        label: KokoroVoices.displayName(for:),
                        width: 240
                    )
                }

                if !DaemonConfig.endpoint.isLoopbackRuntime {
                    SettingsRow(
                        label: L10n.tr("settings.voice.remote_stt_consent"),
                        description: L10n.tr("settings.voice.remote_stt_consent.description")
                    ) {
                        BobeToggle(
                            isOn: Binding(
                                get: { self.remoteSpeechConsent.isEnabled },
                                set: { self.remoteSpeechConsent.setEnabled($0) }
                            ),
                            accessibilityLabel: L10n.tr("settings.voice.remote_stt_consent")
                        )
                    }
                }

                if self.expertMode.isEnabled {
                    SettingsRow(
                        label: L10n.tr("settings.voice.stt_preference"),
                        description: L10n.tr("settings.voice.stt_preference.description")
                    ) {
                        BobeMenuPicker(
                            selection: Binding(
                                get: { self.sttPreference.preference.rawValue },
                                set: { raw in
                                    guard let preference = VoiceSttPreference(rawValue: raw) else {
                                        return
                                    }
                                    self.sttPreference.setPreference(preference)
                                    self.pipeline.refreshSttPreferenceState()
                                }
                            ),
                            options: VoiceSttPreference.allCases.map(\.rawValue),
                            label: {
                                VoiceSttPreference(rawValue: $0)?.label ?? $0
                            },
                            width: 240
                        )
                    }

                    SettingsRow(
                        label: L10n.tr("settings.voice.tts_backend"),
                        description: L10n.tr("settings.voice.tts_backend.description")
                    ) {
                        BobeMenuPicker(
                            selection: Binding(
                                get: { self.ttsPreference.backend.rawValue },
                                set: { raw in
                                    guard let backend = VoiceTtsBackend(rawValue: raw) else { return }
                                    self.ttsPreference.setBackend(backend)
                                }
                            ),
                            options: VoiceTtsBackend.allCases.map(\.rawValue),
                            label: { self.backendLabel($0) },
                            width: 240
                        )
                    }
                }

                SettingsRow(
                    label: L10n.tr("settings.voice.speed"),
                    description: L10n.tr("settings.voice.speed.description")
                ) {
                    HStack(spacing: 12) {
                        Slider(
                            value: self.store.binding(\.voiceSpeed, fallback: 1.0),
                            in: 0.5 ... 2.0,
                            step: 0.05
                        )
                        .frame(width: 180)
                        .accessibilityLabel(L10n.tr("settings.voice.speed"))
                        .accessibilityHint(L10n.tr("settings.voice.speed.description"))
                        .accessibilityValue(String(format: "%.2f×", Double(self.store.settings?.voiceSpeed ?? 1.0)))
                        Text(String(format: "%.2f×", Double(self.store.settings?.voiceSpeed ?? 1.0)))
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundStyle(self.theme.colors.textMuted)
                            .frame(width: 56, alignment: .trailing)
                            .accessibilityHidden(true)
                    }
                }

                SettingsRow(
                    label: L10n.tr("settings.voice.partial_caption"),
                    description: L10n.tr("settings.voice.partial_caption.description")
                ) {
                    BobeToggle(
                        isOn: self.store.binding(\.voiceShowPartialCaption, fallback: true),
                        accessibilityLabel: L10n.tr("settings.voice.partial_caption")
                    )
                }
            }
        }
    }

    private var modelsSection: some View {
        CollapsibleSection(
            title: L10n.tr("settings.voice.section.models"),
            icon: "internaldrive",
            description: self.modelsSummary
        ) {
            VStack(alignment: .leading, spacing: 14) {
                if self.expertMode.isEnabled,
                   self.ttsPreference.backend == .clientSupertonic {
                    VoiceModelCard(
                        name: L10n.tr("settings.voice.model.supertonic.name"),
                        purpose: L10n.tr("settings.voice.model.supertonic.purpose"),
                        sizeHint: L10n.tr("settings.voice.model.supertonic.size"),
                        location: "~/.cache/fluidaudio/Models/supertonic-3/",
                        status: self.clientTtsInstalled ? .installed : .missing,
                        onInstall: { await self.prepareClientTts() },
                        onUninstall: nil
                    )
                }

                VoiceModelCard(
                    name: self.expertMode.isEnabled
                        ? L10n.tr("settings.voice.model.tts.name_expert")
                        : L10n.tr("settings.voice.model.tts.name"),
                    purpose: L10n.tr("settings.voice.model.tts.purpose"),
                    sizeHint: L10n.tr("settings.voice.model.tts.size"),
                    location: self.expertMode.isEnabled && DaemonConfig.endpoint.daemonFilesystemIsLocalToApp
                        ? VoiceModelPaths.kokoro
                        : nil,
                    status: self.kokoroStatus,
                    onInstall: { await self.installSingle(.daemonTts) },
                    onUninstall: nil
                )

                VoiceModelCard(
                    name: self.expertMode.isEnabled
                        ? L10n.tr("settings.voice.model.stt_en.name_expert")
                        : L10n.tr("settings.voice.model.stt_en.name"),
                    purpose: L10n.tr("settings.voice.model.stt_en.purpose"),
                    sizeHint: L10n.tr("settings.voice.model.stt_en.size"),
                    location: self.expertMode.isEnabled
                        ? VoiceModelPaths.parakeet
                        : nil,
                    status: self.parakeetStatus,
                    onInstall: { await self.installSingle(.clientStt(language: "en")) },
                    onUninstall: {
                        await self.uninstall(
                            path: VoiceModelPaths.parakeet,
                            sttLanguage: "en"
                        )
                    }
                )

                VoiceModelCard(
                    name: self.expertMode.isEnabled
                        ? L10n.tr("settings.voice.model.stt_multi.name_expert")
                        : L10n.tr("settings.voice.model.stt_multi.name"),
                    purpose: L10n.tr("settings.voice.model.stt_multi.purpose"),
                    sizeHint: L10n.tr("settings.voice.model.stt_multi.size"),
                    location: self.expertMode.isEnabled
                        ? VoiceModelPaths.nemotron
                        : nil,
                    status: self.nemotronStatus,
                    onInstall: {
                        let language = self.store.settings?.voiceSttLanguage ?? "zh"
                        await self.installSingle(.clientStt(
                            language: language == "en" ? "zh" : language
                        ))
                    },
                    onUninstall: {
                        await self.uninstall(
                            path: VoiceModelPaths.nemotron,
                            sttLanguage: "zh"
                        )
                    }
                )

                if self.expertMode.isEnabled {
                    HStack(alignment: .top, spacing: 6) {
                        Image(systemName: "info.circle")
                            .font(.system(size: 10))
                            .foregroundStyle(self.theme.colors.textMuted)
                        Text(L10n.tr("settings.voice.section.models.location_note"))
                            .bobeTextStyle(.helper)
                            .foregroundStyle(self.theme.colors.textMuted)
                            .fixedSize(horizontal: false, vertical: true)
                        Spacer(minLength: 0)
                    }
                    .padding(.top, 4)
                }

                HStack(spacing: 8) {
                    Button(self.isReinstalling
                        ? L10n.tr("settings.voice.reinstalling")
                        : L10n.tr("settings.voice.reinstall")
                    ) {
                        Task { await self.reinstall() }
                    }
                    .bobeButton(.primary, size: .small)
                    .disabled(self.isReinstalling)
                    .accessibilityLabel(L10n.tr("settings.voice.reinstall.accessibility"))
                    Text(self.isReinstalling
                        ? L10n.tr("settings.voice.reinstall.running")
                        : L10n.tr("settings.voice.reinstall.description")
                    )
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.top, 4)
            }
        }
    }

    private var kokoroStatus: VoiceModelCard.Status {
        guard let progress = self.installStatus?.models.first(where: { $0.kind == VoiceWire.modelKindTts }) else {
            return .unknown
        }
        if self.installStatus?.installed.tts == true {
            return .installed
        }
        if progress.status.starts(with: "downloading") || self.isReinstalling {
            return .downloading(
                bytesDownloaded: progress.bytesDownloaded,
                bytesTotal: progress.bytesTotal,
                percent: progress.percent
            )
        }
        if progress.status == "failed" {
            return .failed(self.installStatus?.error ?? progress.status)
        }
        return .missing
    }

    private var parakeetStatus: VoiceModelCard.Status {
        self.cardStatus(for: FluidAudioModelPresence.self, language: "en")
    }

    private var nemotronStatus: VoiceModelCard.Status {
        self.cardStatus(for: FluidAudioNemotronModelPresence.self, language: "zh")
    }

    private func cardStatus<P: FluidAudioPresence>(
        for presence: P.Type,
        language: String
    ) -> VoiceModelCard.Status {
        let wantsEnglish = language == "en"
        let activeLanguage = self.store.settings?.voiceSttLanguage ?? "en"
        let activeMatches = (activeLanguage == "en") == wantsEnglish
        let downloadMatches = self.pipeline.sttDownloadLanguage.map {
            ($0 == "en") == wantsEnglish
        } ?? false

        if downloadMatches {
            return .downloading(
                bytesDownloaded: 0,
                bytesTotal: nil,
                percent: self.pipeline.sttLoadProgress?.percent
            )
        }
        if activeMatches {
            switch self.pipeline.sttStatus {
            case .ready: return .installed
            case let .failed(message): return .failed(message)
            case .downloading:
                return .downloading(bytesDownloaded: 0, bytesTotal: nil, percent: nil)
            case .notLoaded: break
            }
        }
        return P.isInstalled() ? .installed : .missing
    }

    private var modelsSummary: String {
        switch self.pipeline.readiness {
        case .ready:
            return L10n.tr("settings.voice.installed_label")
        case .installing, .modelsMissing, .failed:
            let selectedTtsReady = switch self.ttsPreference.backend {
            case .serverKokoro: self.installStatus?.installed.tts ?? false
            case .clientSupertonic: self.clientTtsInstalled
            }
            let sttReady = self.pipeline.sttStatus == .ready
            if selectedTtsReady || sttReady {
                return L10n.tr("settings.voice.partial_label")
            }
            return L10n.tr("settings.voice.missing_label")
        case .disabledByUser, .permissionMissing, .preparing:
            return L10n.tr("settings.voice.missing_label")
        }
    }

    private var voiceLanguage: String {
        self.store.settings?.voiceSttLanguage ?? "en"
    }

    private var languageAdvice: VoiceLanguageAdvice? {
        VoiceLanguageAdvice.advice(
            language: self.voiceLanguage,
            backend: self.ttsPreference.backend,
            persona: self.store.settings?.voicePersona ?? VoiceWire.defaultPersona
        )
    }

    private func adviceMessage(_ advice: VoiceLanguageAdvice) -> String {
        let language = VoiceLanguages.displayName(for: self.voiceLanguage)
        switch advice {
        case let .engine(best):
            return L10n.tr(
                "settings.voice.advice.engine_format",
                self.ttsPreference.backend.shortName, language, best.shortName
            )
        case .persona:
            return L10n.tr("settings.voice.advice.persona_format", language)
        }
    }

    private func apply(_ advice: VoiceLanguageAdvice) {
        withAnimation(BobeMotion.standard) {
            switch advice {
            case let .engine(backend):
                self.ttsPreference.setBackend(backend)
                self.clientTtsInstalled = FluidAudioSupertonicModelPresence.isInstalled()
            case let .persona(slot):
                self.store.binding(\.voicePersona, fallback: VoiceWire.defaultPersona).wrappedValue = slot
            }
        }
    }

    private func backendLabel(_ raw: String) -> String {
        guard let backend = VoiceTtsBackend(rawValue: raw) else { return raw }
        let others = VoiceTtsBackend.allCases.filter { $0 != backend }
        let onlyChoice = backend.speaks(self.voiceLanguage) && !others.contains { $0.speaks(self.voiceLanguage) }
        guard backend == VoiceTtsBackend.best(for: self.voiceLanguage), onlyChoice else { return backend.label }
        return L10n.tr(
            "settings.voice.tts_backend.best_for_format",
            backend.label, VoiceLanguages.displayName(for: self.voiceLanguage)
        )
    }

    private func loadAll() async {
        self.remoteSpeechConsent.refresh()
        await self.store.loadIfNeeded()
        await self.pipeline.refreshDaemonState()
        self.clientTtsInstalled = FluidAudioSupertonicModelPresence.isInstalled()
    }

    private func prepareClientTts() async {
        do {
            try await self.pipeline.clientTtsEngine.prepare()
            self.clientTtsInstalled = true
            self.installError = nil
        } catch {
            self.installError = error.localizedDescription
        }
    }

    private func refreshInstallStatus() async {
        await self.pipeline.refreshDaemonState()
    }

    private func reinstall() async {
        VoiceSetupPreference.setDeferred(false)
        self.isReinstalling = true
        defer { self.isReinstalling = false }
        self.installError = nil
        do {
            try await self.pipeline.prepareSelectedTts()
        } catch {
            self.installError = error.localizedDescription
            return
        }
        Task { @MainActor in
            await self.pipeline.ensureSttLoaded()
        }
        self.statusPollTask?.cancel()
        self.statusPollTask = Task {
            for _ in 0 ..< 600 where !Task.isCancelled {
                try? await Task.sleep(for: .milliseconds(500))
                await self.refreshInstallStatus()
                if self.installStatus?.isTerminal == true {
                    return
                }
            }
        }
    }

    private func installSingle(_ target: VoiceInstallTarget) async {
        VoiceSetupPreference.setDeferred(false)
        self.installError = nil
        do {
            switch target {
            case .daemonTts:
                try await self.pipeline.startDaemonTtsInstall()
            case let .clientStt(language):
                await self.pipeline.ensureSttLoaded(for: language)
            }
        } catch {
            self.installError = error.localizedDescription
        }
        await self.refreshInstallStatus()
    }

    private func uninstall(path: String, sttLanguage: String? = nil) async {
        if let sttLanguage {
            await self.pipeline.unloadSttModel(for: sttLanguage)
        }
        let expanded = NSString(string: path).expandingTildeInPath
        let url = URL(fileURLWithPath: expanded, isDirectory: true)
        if FileManager.default.fileExists(atPath: url.path) {
            try? FileManager.default.removeItem(at: url)
        }
        await self.refreshInstallStatus()
    }
}

private struct VoiceLanguageAdviceStrip: View {
    let message: String
    let action: () -> Void

    @Environment(\.theme) private var theme

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "sparkle")
                .font(.system(size: 11, weight: .bold))
                .foregroundStyle(self.theme.colors.primary)
            Text(self.message)
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.text)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 8)
            Button(L10n.tr("settings.voice.advice.switch"), action: self.action)
                .bobeButton(.primary, size: .small)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(
            RoundedRectangle(cornerRadius: BobeRadius.control, style: .continuous)
                .fill(self.theme.accentSoft)
        )
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("settings.voice.language-advice")
    }
}
