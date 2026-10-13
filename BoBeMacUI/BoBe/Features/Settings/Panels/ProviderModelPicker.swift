import SwiftUI

private enum ProviderModelPickerMetrics {
    static let width: CGFloat = 380
    static let rowHeight: CGFloat = 44
    static let rowSpacing: CGFloat = 2
}

struct ProviderModelPicker: View {
    let profiles: [ProviderProfileWire]
    let initial: WorkloadRouteWire
    let selection: EndpointSelection
    let showsReasoning: Bool
    let onChoose: (WorkloadRouteWire, ProviderModelInfo) -> Void
    @Environment(\.theme) private var theme
    @State private var profile: String
    @State private var model: String
    @State private var reasoning: String
    @State private var query = ""
    @State private var models: [ProviderModelInfo] = []
    @State private var loading = false
    @State private var error: String?
    @State private var hoveredModel: String?
    @Namespace private var profileSelection
    @FocusState private var focusedModel: String?

    init(
        profiles: [ProviderProfileWire],
        initial: WorkloadRouteWire,
        selection: EndpointSelection,
        showsReasoning: Bool = true,
        onChoose: @escaping (WorkloadRouteWire, ProviderModelInfo) -> Void
    ) {
        self.profiles = profiles
        self.initial = initial
        self.selection = selection
        self.showsReasoning = showsReasoning
        self.onChoose = onChoose
        self._profile = State(initialValue: initial.profile)
        self._model = State(initialValue: initial.model ?? "")
        self._reasoning = State(initialValue: initial.reasoningEffort ?? "")
    }

    private var selectedModel: ProviderModelInfo? { self.models.first { $0.id == self.model } }
    private var pending: Bool { self.profiles.first { $0.id == self.profile }?.restartRequired == true }
    private var choices: [ProviderModelInfo] {
        self.models.filter {
            self.query.isEmpty || $0.name.localizedCaseInsensitiveContains(self.query)
                || $0.id.localizedCaseInsensitiveContains(self.query)
        }
    }

    private var reasoningEfforts: [String] {
        guard self.showsReasoning, let efforts = self.selectedModel?.supportedReasoningEfforts, efforts.count > 1 else {
            return []
        }
        return efforts
    }

    private var canChoose: Bool {
        self.selectedModel != nil
            && !self.pending && !self.loading && EndpointStore.shared.selection == self.selection
    }

    private var listHeight: CGFloat {
        let rows = max(1, min(self.choices.count, 5))
        return CGFloat(rows) * ProviderModelPickerMetrics.rowHeight
            + CGFloat(rows - 1) * ProviderModelPickerMetrics.rowSpacing
            + (self.choices.count > 5 ? ProviderModelPickerMetrics.rowHeight * 0.45 : 0)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            self.header
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(self.theme.colors.textMuted)
                TextField(L10n.tr("settings.providers.search_models"), text: self.$query)
                    .textFieldStyle(.plain)
                    .bobeTextStyle(.inputField)
            }
            .padding(.horizontal, 12)
            .frame(height: 34)
            .bobeWell(cornerRadius: BobeRadius.control)
            self.modelList
            if let error {
                SettingsErrorBanner(message: error)
            }
            if !self.reasoningEfforts.isEmpty {
                HStack(spacing: 10) {
                    Text(L10n.tr("settings.providers.reasoning"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                        .help(L10n.tr("settings.providers.reasoning.help"))
                    BobeSegmented(
                        selection: self.$reasoning,
                        options: [""] + self.reasoningEfforts,
                        title: { effort in
                            effort.isEmpty ? L10n.tr("settings.providers.automatic") : ProviderModelInfo.effortLabel(effort)
                        },
                        height: 28
                    )
                }
                .transition(.bobeRise)
                HStack {
                    Spacer()
                    Button(L10n.tr("settings.providers.choose_model")) { self.choose() }
                        .bobeButton(.primary, size: .small)
                        .keyboardShortcut(.defaultAction)
                        .disabled(!self.canChoose)
                }
                .transition(.bobeRise)
            }
        }
        .padding(16)
        .frame(width: ProviderModelPickerMetrics.width)
        .background(self.theme.ground)
        .animation(BobeMotion.standard, value: self.reasoningEfforts)
        .animation(BobeMotion.standard, value: self.choices.count)
        .animation(BobeMotion.fade, value: self.loading)
        .task(id: self.profile) { await self.load() }
        .onMoveCommand { direction in
            let choices = self.choices
            guard !choices.isEmpty, direction == .up || direction == .down else { return }
            let current = choices.firstIndex { $0.id == self.focusedModel }
            let index = current.map { direction == .down ? min($0 + 1, choices.count - 1) : max($0 - 1, 0) } ?? 0
            self.focusedModel = choices[index].id
        }
        .onChange(of: self.profile) { _, _ in
            self.model = ""
            self.reasoning = ""
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            if self.profiles.count > 1 {
                ScrollView(.horizontal) {
                    HStack(spacing: 6) {
                        ForEach(self.profiles) { provider in
                            self.profileChip(provider)
                        }
                    }
                    .padding(.vertical, 2)
                }
                .scrollIndicators(.never)
            } else if let provider = self.profiles.first {
                ProviderMark(kind: provider.kind, size: 26)
                Text(provider.kind.displayName)
                    .bobeTextStyle(.rowTitle)
                    .foregroundStyle(self.theme.colors.text)
                    .lineLimit(1)
                Spacer(minLength: 0)
            }
            Button {
                Task { await self.load() }
            } label: {
                Image(systemName: "arrow.clockwise")
            }
            .bobeButton(.ghost, size: .small)
            .disabled(self.loading || self.pending)
            .help(L10n.tr("settings.providers.load_models"))
            .accessibilityLabel(L10n.tr("settings.providers.load_models"))
        }
    }

    private func profileChip(_ provider: ProviderProfileWire) -> some View {
        let isSelected = provider.id == self.profile
        return Button {
            self.profile = provider.id
        } label: {
            HStack(spacing: 6) {
                ProviderMark(kind: provider.kind, size: 20)
                Text(provider.kind.displayName)
                    .bobeTextStyle(.caption)
                    .foregroundStyle(isSelected ? self.theme.colors.text : self.theme.colors.textMuted)
                    .lineLimit(1)
            }
            .padding(.leading, 5)
            .padding(.trailing, 10)
            .frame(height: 30)
            .background {
                if isSelected {
                    Capsule(style: .circular)
                        .fill(self.theme.colors.surface)
                        .overlay(Capsule(style: .circular).strokeBorder(self.theme.colors.primary.opacity(0.55), lineWidth: 1.5))
                        .matchedGeometryEffect(id: "model.profile.selection", in: self.profileSelection)
                } else {
                    Capsule(style: .circular).fill(self.theme.well.opacity(0.6))
                }
            }
            .contentShape(Capsule(style: .circular))
        }
        .buttonStyle(BobePressStyle())
        .animation(BobeMotion.snappy, value: isSelected)
        .accessibilityLabel("\(provider.kind.displayName) · \(provider.id)")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    @ViewBuilder
    private var modelList: some View {
        if self.loading {
            BobeSpinner(size: 16)
                .frame(maxWidth: .infinity)
                .frame(height: ProviderModelPickerMetrics.rowHeight * 2)
        } else if self.pending {
            Label(L10n.tr("settings.providers.pending"), systemImage: "arrow.clockwise")
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.warning)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 12)
        } else if self.choices.isEmpty {
            if self.error == nil {
                Text(L10n.tr("settings.providers.no_matching_models"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .frame(maxWidth: .infinity)
                    .frame(height: ProviderModelPickerMetrics.rowHeight)
            }
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: ProviderModelPickerMetrics.rowSpacing) {
                    ForEach(self.choices) { choice in
                        self.modelRow(choice)
                    }
                }
            }
            .scrollIndicators(.never)
            .frame(height: self.listHeight)
        }
    }

    private func modelRow(_ choice: ProviderModelInfo) -> some View {
        let isSelected = self.model == choice.id
        return Button {
            if self.model != choice.id {
                self.model = choice.id
                self.reasoning = ""
            }
            if self.reasoningEfforts.isEmpty { self.choose() }
        } label: {
            HStack(spacing: 10) {
                VStack(alignment: .leading, spacing: 1) {
                    Text(choice.name)
                        .bobeTextStyle(.settingsBody)
                        .foregroundStyle(self.theme.colors.text)
                        .lineLimit(1)
                    if choice.name != choice.id {
                        Text(choice.id)
                            .bobeTextStyle(.caption)
                            .foregroundStyle(self.theme.colors.textMuted)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                Spacer(minLength: 8)
                if choice.vision {
                    Image(systemName: "photo")
                        .font(.system(size: 11, weight: .medium))
                        .foregroundStyle(self.theme.colors.textMuted)
                        .help(L10n.tr("settings.providers.images_supported"))
                }
                if let window = choice.contextWindow {
                    Text(window, format: .number.notation(.compactName))
                        .bobeTextStyle(.caption)
                        .monospacedDigit()
                        .foregroundStyle(self.theme.colors.textMuted)
                        .help(L10n.tr("settings.providers.context_window"))
                }
                Image(systemName: "checkmark")
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(self.theme.colors.primary)
                    .opacity(isSelected ? 1 : 0)
            }
            .padding(.horizontal, 12)
            .frame(height: ProviderModelPickerMetrics.rowHeight)
            .background(
                RoundedRectangle(cornerRadius: BobeRadius.chip, style: .continuous)
                    .fill(isSelected ? self.theme.accentSoft : (self.hoveredModel == choice.id ? self.theme.well : .clear))
            )
            .contentShape(RoundedRectangle(cornerRadius: BobeRadius.chip, style: .continuous))
        }
        .buttonStyle(BobePressStyle(scale: 0.985))
        .onHover { hovering in
            if hovering {
                self.hoveredModel = choice.id
            } else if self.hoveredModel == choice.id {
                self.hoveredModel = nil
            }
        }
        .focused(self.$focusedModel, equals: choice.id)
        .accessibilityAddTraits(isSelected ? [.isSelected] : [])
    }

    private func choose() {
        guard self.canChoose, let model = self.selectedModel else { return }
        self.onChoose(WorkloadRouteWire(
            profile: self.profile, model: self.model,
            reasoningEffort: self.reasoning.isEmpty ? nil : self.reasoning
        ), model)
    }

    @MainActor
    private func load() async {
        guard EndpointStore.shared.selection == self.selection else { return }
        let profile = self.profile
        self.models = []
        self.error = nil
        self.loading = false
        guard !self.pending else { return }
        self.loading = true
        defer { if self.profile == profile { self.loading = false } }
        do {
            let result = try await DaemonClient(endpoint: self.selection.endpoint).listModels(profile: profile)
            guard !Task.isCancelled, self.profile == profile, EndpointStore.shared.selection == self.selection else { return }
            self.models = result.models
        } catch {
            guard !Task.isCancelled, self.profile == profile, EndpointStore.shared.selection == self.selection else { return }
            self.error = error.localizedDescription
        }
    }
}
