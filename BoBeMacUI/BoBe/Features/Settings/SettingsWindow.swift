import SwiftUI

enum SettingsCategory: String, CaseIterable, Identifiable {
    case souls, goals, memories
    case userProfiles = "user-profiles"
    case mcpServers = "mcp-servers"
    case engine
    case runtime
    case voice
    case devices
    case appearance, behavior, privacy
    case expert

    var id: String {
        rawValue
    }

    var label: String {
        switch self {
        case .souls: L10n.tr("settings.category.souls")
        case .goals: L10n.tr("settings.category.goals")
        case .memories: L10n.tr("settings.category.memories")
        case .userProfiles: L10n.tr("settings.category.user_profiles")
        case .mcpServers: L10n.tr("settings.category.mcp_servers")
        case .engine: L10n.tr("settings.category.engine")
        case .runtime: L10n.tr("settings.category.runtime")
        case .voice: L10n.tr("settings.category.voice")
        case .devices: L10n.tr("settings.category.devices")
        case .appearance: L10n.tr("settings.category.appearance")
        case .behavior: L10n.tr("settings.category.behavior")
        case .privacy: L10n.tr("settings.category.privacy")
        case .expert: L10n.tr("settings.category.expert")
        }
    }

    var searchTerms: [String] {
        switch self {
        case .souls:
            [self.label, L10n.tr("settings.souls.title")]
        case .goals:
            [self.label, L10n.tr("settings.goals.title")]
        case .memories:
            [self.label, L10n.tr("settings.memories.loading")]
        case .userProfiles:
            [self.label, L10n.tr("settings.user_profiles.title")]
        case .mcpServers:
            [self.label, L10n.tr("settings.mcp.description")]
        case .engine:
            [
                self.label, L10n.tr("settings.engine.description"),
                L10n.tr("settings.providers.title"), L10n.tr("settings.providers.auth.api_key"),
                L10n.tr("settings.providers.auth.oauth"), L10n.tr("settings.agent.jobs.title"),
                L10n.tr("settings.agent.schedules.title"), L10n.tr("settings.agent.tools.title"),
            ] + ProviderKindWire.allCases.map(\.displayName)
        case .runtime:
            [
                self.label,
                L10n.tr("settings.runtime.mode"),
                L10n.tr("settings.runtime.address"),
                L10n.tr("settings.runtime.mode.managed"),
                L10n.tr("settings.runtime.mode.attached"),
                L10n.tr("settings.runtime.mode.remote"),
            ]
        case .voice:
            [self.label, L10n.tr("settings.voice.description")]
        case .devices:
            [
                self.label,
                L10n.tr("settings.devices.phone.title"),
                L10n.tr("settings.devices.phone.pair"),
                L10n.tr("settings.devices.list.title"),
            ]
        case .appearance:
            [self.label, L10n.tr("settings.appearance.theme.description")]
        case .behavior:
            [
                self.label,
                L10n.tr("settings.behavior.capture.title"),
                L10n.tr("settings.behavior.capture.description"),
                L10n.tr("settings.behavior.checkins.description"),
                L10n.tr("settings.behavior.conversation.description"),
            ]
        case .privacy:
            [self.label, L10n.tr("settings.providers.storage"), L10n.tr("settings.providers.tools_network")]
        case .expert:
            [self.label, L10n.tr("settings.expert.description")]
        }
    }

    func matchesSearch(_ query: String) -> Bool {
        let tokens = query.lowercased().split(whereSeparator: \Character.isWhitespace)
        guard !tokens.isEmpty else { return true }
        let haystack = self.searchTerms.joined(separator: " ").lowercased()
        return tokens.allSatisfy(haystack.contains)
    }

    var icon: String {
        switch self {
        case .souls: "sparkles"
        case .goals: "target"
        case .memories: "brain.head.profile"
        case .userProfiles: "person.fill"
        case .mcpServers: "hammer.fill"
        case .engine: "cpu.fill"
        case .runtime: "server.rack"
        case .voice: "waveform"
        case .devices: "ipad.and.iphone"
        case .appearance: "paintpalette.fill"
        case .behavior: "bell.badge.fill"
        case .privacy: "hand.raised.fill"
        case .expert: "wrench.and.screwdriver.fill"
        }
    }

    var tint: Color {
        switch self {
        case .souls: Color(rgb: 0xC6694A)
        case .appearance: Color(rgb: 0xD9698A)
        case .voice: Color(rgb: 0xDD9A2F)
        case .devices: Color(rgb: 0x5A7FA0)
        case .userProfiles: Color(rgb: 0x5F88B0)
        case .goals: Color(rgb: 0x5B8A56)
        case .memories: Color(rgb: 0x8E6BA8)
        case .behavior: Color(rgb: 0x3F9488)
        case .privacy: Color(rgb: 0x64748B)
        case .engine: Color(rgb: 0xB5653A)
        case .mcpServers: Color(rgb: 0xA6832E)
        case .runtime: Color(rgb: 0x6F6862)
        case .expert: Color(rgb: 0x4F4944)
        }
    }

    var lede: String {
        switch self {
        case .souls: L10n.tr("settings.page.souls.lede")
        case .goals: L10n.tr("settings.page.goals.lede")
        case .memories: L10n.tr("settings.page.memories.lede")
        case .userProfiles: L10n.tr("settings.page.user_profiles.lede")
        case .mcpServers: L10n.tr("settings.mcp.description")
        case .engine: L10n.tr("settings.engine.description")
        case .runtime: L10n.tr("settings.page.runtime.lede")
        case .voice: L10n.tr("settings.voice.description")
        case .devices: L10n.tr("settings.page.devices.lede")
        case .appearance: L10n.tr("settings.page.appearance.lede")
        case .behavior: L10n.tr("settings.page.behavior.lede")
        case .privacy: L10n.tr("settings.page.privacy.lede")
        case .expert: L10n.tr("settings.expert.description")
        }
    }
}

enum SettingsCategoryGroup: String, CaseIterable {
    case bobe
    case you
    case initiative
    case engineRoom

    var categories: [SettingsCategory] {
        switch self {
        case .bobe:
            [.souls, .appearance, .voice, .devices]
        case .you:
            [.userProfiles, .goals, .memories]
        case .initiative:
            [.behavior, .privacy]
        case .engineRoom:
            [.engine, .mcpServers, .runtime, .expert]
        }
    }

    var label: String {
        switch self {
        case .bobe: L10n.tr("settings.group.bobe")
        case .you: L10n.tr("settings.group.you")
        case .initiative: L10n.tr("settings.group.initiative")
        case .engineRoom: L10n.tr("settings.group.engine_room")
        }
    }
}

struct SettingsWindow: View {
    let initialCategory: SettingsCategory?

    @State private var selectedCategory: SettingsCategory?
    @State private var searchQuery = ""
    @State private var editCoordinator = SettingsEditCoordinator.shared
    @State private var gaze: CGSize = .zero
    private let themeStore = ThemeStore.shared

    init(initialCategory: SettingsCategory? = nil) {
        self.initialCategory = initialCategory
        self._selectedCategory = State(initialValue: initialCategory)
    }

    private var theme: ThemeConfig {
        self.themeStore.currentTheme
    }

    var body: some View {
        HStack(spacing: 0) {
            SettingsSidebar(
                selection: self.selectedCategory,
                searchQuery: self.$searchQuery,
                gaze: self.gaze,
                onSelect: self.requestCategory
            )
            .frame(width: 244)

            self.stage
        }
        .background(self.theme.ground.ignoresSafeArea())
        .ignoresSafeArea(.container, edges: .top)
        .onContinuousHover { phase in
            if case let .active(location) = phase {
                self.gaze = PointerTracker.gaze(
                    from: CGPoint(x: 44, y: 60),
                    to: CGPoint(x: location.x, y: 120 - location.y),
                    reach: 320
                )
            } else {
                self.gaze = .zero
            }
        }
        .environment(SettingsStore.shared)
        .environment(ExpertMode.shared)
        .environment(ThemeStore.shared)
        .environment(VoicePipeline.shared)
        .environment(\.theme, self.theme)
        .preferredColorScheme(self.theme.isDark ? .dark : .light)
        .frame(minWidth: 760, minHeight: 520)
        .confirmationDialog(
            L10n.tr("settings.unsaved.title"),
            isPresented: self.$editCoordinator.showsConfirmation,
            titleVisibility: .visible
        ) {
            Button(L10n.tr("settings.shared.action.save")) {
                Task { await self.editCoordinator.saveAndContinue() }
            }
            Button(L10n.tr("settings.shared.action.discard"), role: .destructive) {
                self.editCoordinator.discardAndContinue()
            }
            Button(L10n.tr("settings.editor.action.cancel"), role: .cancel) {
                self.editCoordinator.cancelTransition()
            }
        } message: {
            Text(L10n.tr("settings.unsaved.message"))
        }
    }

    private var stage: some View {
        let shape = RoundedRectangle(cornerRadius: 20, style: .continuous)
        return VStack(spacing: 0) {
            if let category = self.selectedCategory {
                SettingsPageHeader(category: category)
                    .id(category)
                    .transition(.bobeRise)
            }
            self.settingsContent
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .id(self.selectedCategory)
                .transition(.bobeRise)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .clipShape(shape)
        .bobeRaised(in: shape, fill: self.theme.colors.background)
        .padding(.vertical, 8)
        .padding(.trailing, 8)
        .animation(BobeMotion.soft, value: self.selectedCategory)
    }

    private func requestCategory(_ category: SettingsCategory?) {
        self.editCoordinator.requestTransition { self.selectedCategory = category }
    }

    @ViewBuilder
    private var settingsContent: some View {
        switch self.selectedCategory {
        case nil:
            SettingsOverview(gaze: self.gaze, onNavigate: self.requestCategory)
        case .souls:
            SoulsEditor()
        case .goals:
            GoalsEditor()
        case .memories:
            MemoriesEditor()
        case .userProfiles:
            UserProfilesEditor()
        case .mcpServers:
            MCPServersPanel()
        case .engine:
            EnginePanel()
        case .runtime:
            RuntimePanel()
        case .voice:
            VoicePanel()
        case .devices:
            DevicesPanel()
        case .appearance:
            AppearancePanel()
        case .behavior:
            BehaviorPanel()
        case .privacy:
            PrivacyPanel()
        case .expert:
            ExpertModePanel()
        }
    }
}

struct SettingsPageHeader: View {
    let category: SettingsCategory
    @Environment(\.theme) private var theme

    var body: some View {
        HStack(spacing: 13) {
            BobeIconTile(systemName: self.category.icon, tint: self.category.tint, size: 36)
            VStack(alignment: .leading, spacing: 1) {
                Text(self.category.label)
                    .bobeTextStyle(.display)
                    .foregroundStyle(self.theme.colors.text)
                Text(self.category.lede)
                    .bobeTextStyle(.body)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .lineLimit(2)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 24)
        .padding(.top, 20)
        .padding(.bottom, 14)
        .overlay(alignment: .bottom) {
            Rectangle()
                .fill(self.theme.hairline.opacity(0.7))
                .frame(height: 1)
        }
    }
}

struct SettingsSidebar: View {
    let selection: SettingsCategory?
    @Binding var searchQuery: String
    let gaze: CGSize
    let onSelect: (SettingsCategory?) -> Void

    @Environment(\.theme) private var theme
    @Namespace private var activePill
    @FocusState private var searchFocused: Bool

    private var allVisibleCategories: [SettingsCategory] {
        SettingsCategoryGroup.allCases.flatMap { self.visibleCategories(in: $0) }
    }

    private func visibleCategories(in group: SettingsCategoryGroup) -> [SettingsCategory] {
        group.categories.filter { $0.matchesSearch(self.searchQuery) }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            self.identity
                .padding(.top, 34)
                .padding(.horizontal, 12)

            self.search
                .padding(.horizontal, 12)
                .padding(.top, 12)
                .padding(.bottom, 6)

            ScrollView {
                VStack(alignment: .leading, spacing: 2) {
                    ForEach(SettingsCategoryGroup.allCases, id: \.self) { group in
                        let visible = self.visibleCategories(in: group)
                        if !visible.isEmpty {
                            Text(group.label)
                                .bobeTextStyle(.sectionLabel)
                                .foregroundStyle(self.theme.colors.textMuted.opacity(0.85))
                                .padding(.horizontal, 10)
                                .padding(.top, 14)
                                .padding(.bottom, 4)
                            ForEach(visible) { category in
                                self.row(for: category)
                            }
                        }
                    }
                    if self.allVisibleCategories.isEmpty {
                        Text(String(format: L10n.tr("settings.window.search.no_results"), self.searchQuery))
                            .bobeTextStyle(.helper)
                            .foregroundStyle(self.theme.colors.textMuted)
                            .padding(.horizontal, 10)
                            .padding(.vertical, 14)
                    }
                }
                .padding(.horizontal, 8)
                .padding(.bottom, 8)
            }
            .scrollBounceBehavior(.basedOnSize)

            Button {
                BobeCommandRouter.shared.checkForUpdates()
            } label: {
                Label(L10n.tr("settings.window.check_updates"), systemImage: "arrow.triangle.2.circlepath")
            }
            .bobeButton(.ghost, size: .small)
            .disabled(!BobeCommandRouter.shared.canCheckForUpdates)
            .padding(.horizontal, 12)
            .padding(.bottom, 14)
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    private var identity: some View {
        Button {
            self.onSelect(nil)
        } label: {
            HStack(spacing: 6) {
                BobeCharacter(
                    mood: .idle,
                    size: 38,
                    palette: self.theme.avatar,
                    gaze: self.gaze
                )
                Text(L10n.tr("overlay.avatar.brand_label"))
                    .font(.system(size: 19, weight: .bold, design: .rounded))
                    .foregroundStyle(self.theme.colors.text)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 4)
            .padding(.vertical, 4)
            .background {
                if self.selection == nil {
                    self.pill
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(BobePressStyle(scale: 0.98))
        .accessibilityLabel(L10n.tr("settings.window.title"))
        .accessibilityIdentifier("settings.sidebar.home")
        .accessibilityAddTraits(self.selection == nil ? .isSelected : [])
    }

    private var pill: some View {
        RoundedRectangle(cornerRadius: 13, style: .continuous)
            .fill(self.theme.colors.surface)
            .shadow(color: self.theme.shadowTint.opacity(self.theme.isDark ? 0.35 : 0.1), radius: 4, x: 1, y: 2)
            .overlay {
                RoundedRectangle(cornerRadius: 13, style: .continuous)
                    .strokeBorder(self.theme.hairline, style: BobeStroke.hairline)
            }
            .matchedGeometryEffect(id: "active", in: self.activePill)
    }

    private var search: some View {
        HStack(spacing: 7) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 12, weight: .medium))
                .foregroundStyle(self.theme.colors.textMuted)
            ZStack(alignment: .leading) {
                if self.searchQuery.isEmpty {
                    Text(L10n.tr("settings.window.search.placeholder"))
                        .font(.system(size: 13))
                        .foregroundStyle(self.theme.colors.textMuted.opacity(0.8))
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
                TextField("", text: self.$searchQuery)
                    .textFieldStyle(.plain)
                    .font(.system(size: 13))
                    .foregroundStyle(self.theme.colors.text)
                    .tint(self.theme.colors.primary)
                    .focused(self.$searchFocused)
                    .accessibilityLabel(L10n.tr("settings.window.search.placeholder"))
                    .accessibilityIdentifier("settings.search")
            }
            if !self.searchQuery.isEmpty {
                Button {
                    self.searchQuery = ""
                } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 12))
                        .foregroundStyle(self.theme.colors.textMuted)
                }
                .buttonStyle(BobePressStyle(scale: 0.85))
                .accessibilityLabel(L10n.tr("settings.editor.action.cancel"))
                .transition(.bobePop())
            }
        }
        .padding(.horizontal, 11)
        .frame(height: 32)
        .bobeWell(in: .bobePill)
        .animation(BobeMotion.snappy, value: self.searchQuery.isEmpty)
    }

    private func row(for category: SettingsCategory) -> some View {
        SettingsSidebarRow(
            category: category,
            isSelected: self.selection == category,
            pill: self.selection == category ? AnyView(self.pill) : nil
        ) {
            withAnimation(BobeMotion.standard) {
                self.onSelect(category)
            }
        }
    }
}

private struct SettingsSidebarRow: View {
    let category: SettingsCategory
    let isSelected: Bool
    let pill: AnyView?
    let action: () -> Void

    @Environment(\.theme) private var theme
    @State private var isHovered = false

    var body: some View {
        Button(action: self.action) {
            HStack(spacing: 10) {
                BobeIconTile(systemName: self.category.icon, tint: self.category.tint, size: 24)
                    .scaleEffect(self.isHovered && !self.isSelected ? 1.08 : 1)
                Text(self.category.label)
                    .font(.system(size: 13.5, weight: self.isSelected ? .semibold : .regular))
                    .foregroundStyle(self.theme.colors.text)
                Spacer(minLength: 0)
            }
            .padding(.horizontal, 8)
            .frame(height: 36)
            .background {
                if let pill {
                    pill
                } else if self.isHovered {
                    RoundedRectangle(cornerRadius: 13, style: .continuous)
                        .fill(self.theme.well.opacity(0.75))
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(BobePressStyle(scale: 0.98))
        .accessibilityIdentifier("settings.sidebar.\(self.category.rawValue)")
        .accessibilityAddTraits(self.isSelected ? .isSelected : [])
        .animation(BobeMotion.snappy, value: self.isHovered)
        .onHover { self.isHovered = $0 }
    }
}

struct SettingsOverview: View {
    var gaze: CGSize = .zero
    var onNavigate: (SettingsCategory) -> Void
    @Environment(\.theme) private var theme

    @State private var goalsActive: Int?
    @State private var soulsCustom: Int?
    @State private var profileKnown: Bool?
    @State private var hasAppeared = false
    @State private var greeted = false

    private var cards: [(category: SettingsCategory, status: String)] {
        [
            (.goals, self.goalsLabel),
            (.souls, self.soulsLabel),
            (.userProfiles, self.profilesLabel),
            (.behavior, L10n.tr("settings.window.overview.row.behavior.label")),
        ]
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 0) {
                BobeCharacter(
                    mood: self.greeted ? .idle : .happy,
                    size: 128,
                    palette: self.theme.avatar,
                    gaze: self.gaze
                )
                .padding(.top, 44)
                .scaleEffect(self.hasAppeared ? 1 : 0.8)
                .opacity(self.hasAppeared ? 1 : 0)

                Text(L10n.tr("settings.window.overview.title"))
                    .font(.system(size: 30, weight: .semibold, design: .rounded))
                    .foregroundStyle(self.theme.colors.text)
                    .padding(.top, 14)

                Text(L10n.tr("settings.window.overview.lede"))
                    .bobeTextStyle(.setupBody)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .multilineTextAlignment(.center)
                    .lineSpacing(3)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: 520)
                    .padding(.top, 8)

                LazyVGrid(
                    columns: [GridItem(.flexible(), spacing: 14), GridItem(.flexible(), spacing: 14)],
                    spacing: 14
                ) {
                    ForEach(Array(self.cards.enumerated()), id: \.element.category) { index, card in
                        OverviewCard(category: card.category, status: card.status) {
                            self.onNavigate(card.category)
                        }
                        .opacity(self.hasAppeared ? 1 : 0)
                        .offset(y: self.hasAppeared ? 0 : 10)
                        .animation(BobeMotion.stagger(index, step: 0.05), value: self.hasAppeared)
                    }
                }
                .frame(maxWidth: 680)
                .padding(.top, 30)

                Label(L10n.tr("settings.window.overview.footer"), systemImage: "lock.fill")
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .multilineTextAlignment(.center)
                    .padding(.top, 26)
                    .padding(.bottom, 40)
            }
            .padding(.horizontal, 40)
            .frame(maxWidth: .infinity, alignment: .center)
        }
        .task { await self.loadCounts() }
        .task {
            withAnimation(BobeMotion.soft) { self.hasAppeared = true }
            try? await Task.sleep(for: .seconds(1.3))
            self.greeted = true
        }
    }

    private var goalsLabel: String {
        switch self.goalsActive {
        case .none, 0:
            L10n.tr("settings.window.overview.row.goals.empty")
        case 1:
            L10n.tr("settings.window.overview.row.goals.known_one")
        case let count?:
            L10n.tr("settings.window.overview.row.goals.known_many_format", count)
        }
    }

    private var soulsLabel: String {
        switch self.soulsCustom {
        case .none, 0:
            L10n.tr("settings.window.overview.row.souls.empty")
        case 1:
            L10n.tr("settings.window.overview.row.souls.known_one")
        case let count?:
            L10n.tr("settings.window.overview.row.souls.known_many_format", count)
        }
    }

    private var profilesLabel: String {
        switch self.profileKnown {
        case .some(true):
            L10n.tr("settings.window.overview.row.profiles.known")
        default:
            L10n.tr("settings.window.overview.row.profiles.empty")
        }
    }

    private func loadCounts() async {
        async let goalsTask = try? await DaemonClient.shared.listGoals()
        async let soulsTask = try? await DaemonClient.shared.listSouls()
        async let profilesTask = try? await DaemonClient.shared.listUserProfiles()

        let goals = await goalsTask
        let souls = await soulsTask
        let profiles = await profilesTask

        self.goalsActive = goals?.activeCount
        self.soulsCustom = souls.map { max(0, $0.count - 1) }
        self.profileKnown = profiles.map { list in list.profiles.contains { !$0.isDefault } }
    }
}

private struct OverviewCard: View {
    let category: SettingsCategory
    let status: String
    let action: () -> Void

    @Environment(\.theme) private var theme
    @State private var isHovered = false

    var body: some View {
        Button(action: self.action) {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    BobeIconTile(systemName: self.category.icon, tint: self.category.tint, size: 34)
                    Spacer()
                    Image(systemName: "arrow.up.right")
                        .font(.system(size: 11, weight: .bold))
                        .foregroundStyle(self.isHovered ? self.theme.colors.primary : self.theme.colors.textMuted.opacity(0.5))
                        .offset(x: self.isHovered ? 2 : 0, y: self.isHovered ? -2 : 0)
                }
                VStack(alignment: .leading, spacing: 3) {
                    Text(self.category.label)
                        .bobeTextStyle(.title)
                        .foregroundStyle(self.theme.colors.text)
                    Text(self.status)
                        .bobeTextStyle(.body)
                        .foregroundStyle(self.theme.colors.textMuted)
                        .multilineTextAlignment(.leading)
                        .lineLimit(3)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .padding(16)
            .frame(maxWidth: .infinity, minHeight: 124, alignment: .topLeading)
            .bobeRaised(cornerRadius: BobeRadius.card, elevation: self.isHovered ? .lifted : .resting)
            .offset(y: self.isHovered ? -2 : 0)
            .contentShape(Rectangle())
        }
        .buttonStyle(BobePressStyle(scale: 0.98))
        .animation(BobeMotion.standard, value: self.isHovered)
        .onHover { self.isHovered = $0 }
    }
}
