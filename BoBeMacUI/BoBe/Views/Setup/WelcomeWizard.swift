import AppKit
import SwiftUI

enum WelcomeStep: Hashable {
    case welcome
    case runtimeLocation
    case engineChoice
    case providerSetup
    case localSetup
    case personalize
    case proactivity
    case permissions
    case voiceSetup
    case physicalBoBe
    case done
}

enum EngineChoice: String, Sendable {
    case provider
    case local
}

struct SetupRoute: Equatable {
    var initialStep: WelcomeStep?
    var completesAfterRuntimeRepair = false
    var initializationState: InitializationStateWire = .providerRequired
    var engineChoice: EngineChoice?

    func step(after step: WelcomeStep) -> WelcomeStep? {
        switch step {
        case .welcome:
            return .runtimeLocation
        case .runtimeLocation:
            guard !self.completesAfterRuntimeRepair else { return nil }
            switch self.initializationState {
            case .providerRequired: return .engineChoice
            case .personalizationRequired: return .personalize
            case .ready: return .permissions
            }
        case .engineChoice:
            return self.engineChoice == .local ? .localSetup : .providerSetup
        case .providerSetup, .localSetup:
            return .personalize
        case .personalize:
            return .proactivity
        case .proactivity:
            return .permissions
        case .permissions:
            return .voiceSetup
        case .voiceSetup:
            return self.initialStep == .voiceSetup ? nil : .physicalBoBe
        case .physicalBoBe:
            return .done
        case .done:
            return nil
        }
    }

    func stepsRemaining(after step: WelcomeStep) -> Int {
        var count = 0
        var cursor = step
        while let next = self.step(after: cursor) {
            count += 1
            cursor = next
        }
        return count
    }
}

enum ProviderSetupError: LocalizedError {
    case externalRestartRequired

    var errorDescription: String? {
        switch self {
        case .externalRestartRequired:
            L10n.tr("setup.provider.restart_external")
        }
    }
}

struct PendingProviderActivation: Equatable {
    var selection: EndpointSelection
    let profileID: String
    let request: ProviderActivationRequest
    var previousIncarnation: UUID?

    func matches(selection: EndpointSelection, profileID: String, request: ProviderActivationRequest) -> Bool {
        self.selection == selection && self.profileID == profileID && self.request == request
    }
}

@Observable
final class SetupNavigation {
    private(set) var currentStep: WelcomeStep
    private(set) var history: [WelcomeStep] = []
    private(set) var isCommitting = false
    private(set) var commitError: String?

    init(initialStep: WelcomeStep) {
        self.currentStep = initialStep
    }

    func commit(_ operation: (() async throws -> Void)?) async -> Bool {
        guard !self.isCommitting else { return false }
        self.commitError = nil
        guard let operation else { return true }
        self.isCommitting = true
        defer { self.isCommitting = false }
        do {
            try await operation()
            return true
        } catch {
            self.commitError = error.localizedDescription
            return false
        }
    }

    func advance(to step: WelcomeStep) {
        self.history.append(self.currentStep)
        withAnimation(BobeMotion.soft) {
            self.currentStep = step
        }
    }

    func goBack() {
        guard !self.isCommitting, let previous = self.history.popLast() else { return }
        self.commitError = nil
        withAnimation(BobeMotion.soft) {
            self.currentStep = previous
        }
    }

    func goBack(after operation: @escaping () async throws -> Void) async {
        guard !self.history.isEmpty, await self.commit(operation) else { return }
        self.goBack()
    }
}

enum WelcomeWizardMetrics {
    static let size = CGSize(width: 800, height: 620)
    static let titlebarHeight: CGFloat = 32
    static let contentHeight: CGFloat = size.height - titlebarHeight
    static let roomWidth: CGFloat = 250
}

struct SetupStepSlide: ViewModifier {
    let offset: CGFloat
    let isActive: Bool

    func body(content: Content) -> some View {
        content
            .offset(x: self.offset)
            .opacity(self.isActive ? 0 : 1)
            .blur(radius: self.isActive ? 4 : 0)
    }
}

struct SetupOrbitBackdrop: View {
    let color: Color

    private struct Ring {
        let radius: CGFloat
        let width: CGFloat
        let share: CGFloat
        let start: Double
    }

    private static let rings = [
        Ring(radius: 118, width: 3, share: 0.62, start: 20),
        Ring(radius: 164, width: 2.4, share: 0.48, start: 140),
        Ring(radius: 214, width: 2, share: 0.7, start: 250),
        Ring(radius: 270, width: 1.6, share: 0.4, start: 60),
    ]

    var body: some View {
        ZStack {
            ForEach(Array(Self.rings.enumerated()), id: \.offset) { _, ring in
                Circle()
                    .trim(from: 0, to: ring.share)
                    .stroke(self.color, style: StrokeStyle(lineWidth: ring.width, lineCap: .round))
                    .frame(width: ring.radius * 2, height: ring.radius * 2)
                    .rotationEffect(.degrees(ring.start))
            }
        }
        .offset(y: -8)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

struct WelcomeWizard: View {
    let onComplete: () -> Void
    var initialStep: WelcomeStep?
    var runtimeRepairIssue: RuntimeIssue?
    var completesAfterRuntimeRepair = false

    @State private var navigation: SetupNavigation
    @State private var engineChoice: EngineChoice? = .provider
    @State private var providerFields = ProviderConfigurationFields()
    @State private var providerAPIKey = ""
    @State private var providerSelection = EndpointStore.shared.selection
    @State private var pendingProviderActivation: PendingProviderActivation?
    @State private var runtimeInitializationState: InitializationStateWire = .providerRequired
    @State private var preferredName = ""
    @State private var firstGoal = ""
    @State private var proactivityLevel: ProactivityLevel = .balanced
    @State private var existingScreenAwareness: Bool?
    @State private var isLeavingLocalSetup = false
    @State private var isRuntimeOperationCritical = false
    @State private var gaze: CGSize = .zero
    @State private var isAwake = false
    private let themeStore = ThemeStore.shared

    init(
        onComplete: @escaping () -> Void,
        initialStep: WelcomeStep? = nil,
        runtimeRepairIssue: RuntimeIssue? = nil,
        completesAfterRuntimeRepair: Bool = false
    ) {
        self.onComplete = onComplete
        self.initialStep = initialStep
        self.runtimeRepairIssue = runtimeRepairIssue
        self.completesAfterRuntimeRepair = completesAfterRuntimeRepair
        self._navigation = State(initialValue: SetupNavigation(initialStep: initialStep ?? .welcome))
    }

    private var currentStep: WelcomeStep {
        self.navigation.currentStep
    }

    var body: some View {
        HStack(spacing: 0) {
            self.companionRoom
                .frame(width: WelcomeWizardMetrics.roomWidth)

            self.stepStage
        }
        .onChange(of: self.navigation.isCommitting) { _, committing in
            SetupWindowManager.shared.setCriticalOperation(committing || self.isRuntimeOperationCritical)
        }
        .onReceive(NotificationCenter.default.publisher(for: .bobeEndpointIdentityChanged)) { _ in
            let current = EndpointStore.shared.selection
            let changedAuthority = current.profileID != self.providerSelection.profileID
                || current.authorityID != self.providerSelection.authorityID
            self.providerSelection = current
            guard changedAuthority else { return }
            self.providerAPIKey = ""
            self.providerFields = ProviderConfigurationFields()
            self.pendingProviderActivation = nil
        }
        .background(self.themeStore.currentTheme.ground.ignoresSafeArea())
        .ignoresSafeArea(.container, edges: .top)
        .frame(width: WelcomeWizardMetrics.size.width, height: WelcomeWizardMetrics.contentHeight)
        .onContinuousHover { phase in
            if case let .active(location) = phase {
                self.gaze = PointerTracker.gaze(
                    from: CGPoint(x: WelcomeWizardMetrics.roomWidth / 2, y: 0),
                    to: CGPoint(x: location.x, y: WelcomeWizardMetrics.size.height * 0.42 - location.y),
                    reach: 300
                )
            } else {
                self.gaze = .zero
            }
        }
        .task {
            try? await Task.sleep(for: .seconds(0.5))
            self.isAwake = true
        }
        .environment(\.theme, self.themeStore.currentTheme)
        .preferredColorScheme(self.themeStore.preferredColorScheme)
        .confirmationDialog(
            L10n.tr("setup.local.leave.title"),
            isPresented: self.$isLeavingLocalSetup,
            titleVisibility: .visible
        ) {
            Button(L10n.tr("setup.local.leave.keep")) { self.finishGoingBack(cancelInstall: false) }
            Button(L10n.tr("setup.local.leave.cancel"), role: .destructive) {
                self.finishGoingBack(cancelInstall: true)
            }
            Button(L10n.tr("settings.editor.action.cancel"), role: .cancel) {}
        } message: {
            Text(L10n.tr("setup.local.leave.message"))
        }
    }

    private var route: SetupRoute {
        SetupRoute(
            initialStep: self.initialStep,
            completesAfterRuntimeRepair: self.completesAfterRuntimeRepair,
            initializationState: self.runtimeInitializationState,
            engineChoice: self.engineChoice
        )
    }

    private var stepNumber: Int {
        self.navigation.history.count + 1
    }

    private var stepCount: Int {
        self.stepNumber + self.route.stepsRemaining(after: self.currentStep)
    }

    private var progress: Double? {
        guard self.stepCount > 1 else { return nil }
        return Double(self.stepNumber) / Double(self.stepCount)
    }

    private var screenAwarenessOn: Bool {
        if self.navigation.history.contains(.proactivity) {
            return self.proactivityLevel.settings.captureEnabled
        }
        return self.existingScreenAwareness ?? true
    }

    private var companionMood: BobeMood {
        guard self.isAwake else { return .asleep }
        if self.navigation.isCommitting || self.isRuntimeOperationCritical { return .working }
        switch self.currentStep {
        case .welcome: return .idle
        case .runtimeLocation, .engineChoice, .providerSetup, .localSetup: return .idle
        case .personalize: return .listening
        case .proactivity:
            switch self.proactivityLevel {
            case .quiet: return .asleep
            case .balanced: return .idle
            case .proactive: return .eager
            }
        case .permissions: return .watching
        case .voiceSetup: return .listening
        case .physicalBoBe: return .eager
        case .done: return .happy
        }
    }

    private var companionRoom: some View {
        let theme = self.themeStore.currentTheme
        return ZStack {
            SetupOrbitBackdrop(color: theme.avatar.orbit)
                .opacity(theme.isDark ? 0.25 : 0.16)

            VStack(spacing: 14) {
                BobeCharacter(
                    mood: self.companionMood,
                    size: 136,
                    palette: theme.avatar,
                    gaze: self.gaze,
                    voiceLevel: 0.35,
                    wantsAttention: self.currentStep == .done,
                    progress: self.progress
                )
                .animation(BobeMotion.soft, value: self.progress)

                Text(L10n.tr("overlay.avatar.brand_label"))
                    .font(.system(size: 26, weight: .bold, design: .rounded))
                    .foregroundStyle(theme.colors.text)

                if self.stepCount > 1 {
                    Text(
                        L10n.tr(
                            "setup.wizard.progress.value_format",
                            self.stepNumber,
                            self.stepCount
                        )
                    )
                    .bobeTextStyle(.caption)
                    .monospacedDigit()
                    .foregroundStyle(theme.colors.textMuted)
                    .contentTransition(.numericText())
                    .animation(BobeMotion.standard, value: self.currentStep)
                    .accessibilityLabel(L10n.tr("setup.wizard.progress"))
                }
            }
            .offset(y: -8)
        }
        .frame(maxHeight: .infinity)
        .overlay(alignment: .bottomLeading) {
            if self.canGoBack {
                Button(action: self.goBack) {
                    Label(L10n.tr("setup.wizard.back"), systemImage: "chevron.left")
                }
                .bobeButton(.ghost, size: .small)
                .keyboardShortcut("[", modifiers: .command)
                .accessibilityLabel(L10n.tr("setup.wizard.back.accessibility"))
                .accessibilityIdentifier("setup.wizard.back")
                .padding(14)
                .transition(.bobePop(anchor: .bottomLeading))
            }
        }
        .animation(BobeMotion.standard, value: self.canGoBack)
    }

    private var stepStage: some View {
        let theme = self.themeStore.currentTheme
        let shape = RoundedRectangle(cornerRadius: 20, style: .continuous)
        return VStack(spacing: 0) {
            self.stepContent
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .padding(.horizontal, 30)
                .padding(.top, 30)
                .padding(.bottom, 22)
                .transition(self.stepTransition)
                .id(self.currentStep)
            if let commitError = self.navigation.commitError {
                Label(commitError, systemImage: "exclamationmark.triangle.fill")
                    .bobeTextStyle(.helper)
                    .foregroundStyle(theme.colors.error)
                    .padding(.horizontal, 30)
                    .padding(.bottom, 12)
                    .transition(.bobeRise)
            }
        }
        .overlay {
            if self.navigation.isCommitting {
                ZStack {
                    theme.colors.background.opacity(0.78)
                    BobeSpinner(size: 22, lineWidth: 2.5)
                }
                .transition(.opacity)
            }
        }
        .clipShape(shape)
        .bobeRaised(in: shape, fill: theme.colors.background)
        .padding(.vertical, 8)
        .padding(.trailing, 8)
        .animation(BobeMotion.fade, value: self.navigation.isCommitting)
        .animation(BobeMotion.standard, value: self.navigation.commitError)
    }

    @ViewBuilder
    private var stepContent: some View {
        switch self.currentStep {
        case .welcome:
            WelcomeValueStepView(onContinue: { self.advance() })
        case .runtimeLocation:
            RuntimeLocationStepView(
                onContinue: { initializationState in
                    self.runtimeInitializationState = initializationState
                    self.advance()
                },
                onCriticalOperationChange: { critical in
                    self.isRuntimeOperationCritical = critical
                    SetupWindowManager.shared.setCriticalOperation(critical)
                },
                initialProfile: self.runtimeRepairIssue == nil ? nil : EndpointStore.shared.profile,
                initialIssue: self.runtimeRepairIssue
            )
        case .engineChoice:
            EngineChoiceStepView(selection: self.$engineChoice) { self.advance() }
        case .providerSetup:
            ProviderSetupStepView(
                fields: self.$providerFields,
                apiKey: self.$providerAPIKey,
                onContinue: { self.advance() },
                onCriticalOperationChange: { critical in
                    self.isRuntimeOperationCritical = critical
                    SetupWindowManager.shared.setCriticalOperation(critical || self.navigation.isCommitting)
                }
            )
        case .localSetup:
            LocalSetupStepView(
                onContinue: { self.advance() },
                onCriticalOperationChange: { critical in
                    self.isRuntimeOperationCritical = critical
                    SetupWindowManager.shared.setCriticalOperation(critical || self.navigation.isCommitting)
                }
            )
        case .personalize:
            PersonalizeStepView(
                preferredName: self.$preferredName,
                firstGoal: self.$firstGoal,
                onContinue: { self.advance() }
            )
        case .proactivity:
            ProactivityStepView(
                selection: self.$proactivityLevel,
                onContinue: { self.advance() }
            )
        case .permissions:
            PermissionsStepView(
                screenContextEnabled: self.screenAwarenessOn,
                onContinue: { self.advance() }
            )
        case .voiceSetup:
            VoiceSetupStepView(onContinue: { self.advance() })
        case .physicalBoBe:
            PhysicalBoBeSetupStepView(onContinue: { self.advance() })
        case .done:
            DoneStepView(
                firstGoal: self.firstGoal,
                onLaunch: self.onComplete
            )
        }
    }

    private var canGoBack: Bool {
        self.currentStep != .done
            && !self.navigation.history.isEmpty
            && !self.isRuntimeOperationCritical
            && !self.navigation.isCommitting
    }

    private var stepTransition: AnyTransition {
        if BobeMotion.isReduced {
            return .opacity
        }
        return .asymmetric(
            insertion: .modifier(
                active: SetupStepSlide(offset: 26, isActive: true),
                identity: SetupStepSlide(offset: 0, isActive: false)
            ),
            removal: .modifier(
                active: SetupStepSlide(offset: -18, isActive: true),
                identity: SetupStepSlide(offset: 0, isActive: false)
            )
        )
    }

    private func advance() {
        Task {
            let commit: (() async throws -> Void)? = self.requiresCommit
                ? { try await self.commitCurrentStep() }
                : nil
            guard await self.navigation.commit(commit) else { return }
            guard let next = self.route.step(after: self.currentStep) else {
                self.onComplete()
                return
            }
            self.navigation.advance(to: next)
        }
    }

    private var requiresCommit: Bool {
        switch self.currentStep {
        case .providerSetup, .localSetup, .personalize, .proactivity: true
        case .runtimeLocation: self.route.step(after: .runtimeLocation) == .permissions
        default: false
        }
    }

    private func commitCurrentStep() async throws {
        switch self.currentStep {
        case .runtimeLocation:
            self.existingScreenAwareness = try? await DaemonClient.shared.getSettings().captureEnabled
        case .providerSetup:
            try await self.activateProvider(
                profileId: self.providerFields.profileID,
                request: self.providerFields.activation(apiKey: self.providerAPIKey)
            )
            self.providerAPIKey = ""
        case .localSetup:
            let models = LocalRuntimeModels.setup
            try await self.activateProvider(profileId: models.profileID, request: models.activationRequest)
        case .personalize:
            try await self.persistPersonalization()
            _ = try await DaemonClient.shared.advanceInitialization(to: .ready)
        case .proactivity:
            try await self.proactivityLevel.apply { try await DaemonClient.shared.updateSettings($0) }
        default:
            return
        }
    }

    private func activateProvider(
        profileId: String,
        request: ProviderActivationRequest
    ) async throws {
        let selection = EndpointStore.shared.selection
        let client = DaemonClient(endpoint: selection.endpoint)
        if let pending = self.pendingProviderActivation,
           pending.matches(selection: selection, profileID: profileId, request: request) {
            try await self.finishProviderActivation(pending)
            return
        }
        self.pendingProviderActivation = nil
        let previousIncarnation = try await client.readiness().runtimeIncarnationId
        let result = try await client.activateProvider(profileId, request: request)
        guard EndpointStore.shared.selection == selection else { throw CancellationError() }
        let committed = PendingProviderActivation(
            selection: selection, profileID: profileId, request: request,
            previousIncarnation: result.restartRequired ? previousIncarnation : nil
        )
        self.pendingProviderActivation = committed
        try await self.finishProviderActivation(committed)
    }

    private func finishProviderActivation(_ committed: PendingProviderActivation) async throws {
        var committed = committed
        if let previous = committed.previousIncarnation {
            let client = DaemonClient(endpoint: committed.selection.endpoint)
            let current = try await client.readiness().runtimeIncarnationId
            guard EndpointStore.shared.selection == committed.selection else { throw CancellationError() }
            if current == previous {
                guard committed.selection.endpoint.managesLocalProcess else {
                    throw ProviderSetupError.externalRestartRequired
                }
                try await BackendService.shared.userRestart()
            }
            let restarted = EndpointStore.shared.selection
            guard restarted.profileID == committed.selection.profileID,
                  restarted.authorityID == committed.selection.authorityID
            else { throw CancellationError() }
            committed.selection = restarted
            committed.previousIncarnation = nil
            self.pendingProviderActivation = committed
        }
        let client = DaemonClient(endpoint: committed.selection.endpoint)
        _ = try await client.advanceInitialization(to: .personalizationRequired)
        guard EndpointStore.shared.selection == committed.selection else { throw CancellationError() }
        self.pendingProviderActivation = nil
    }

    private func persistPersonalization() async throws {
        let trimmedName = self.preferredName.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmedName.isEmpty {
            let profiles = try await DaemonClient.shared.listUserProfiles().profiles
            if !profiles.contains(where: { $0.name.caseInsensitiveCompare(trimmedName) == .orderedSame }) {
                _ = try await DaemonClient.shared.createUserProfile(
                    UserProfileCreateRequest(
                        name: trimmedName,
                        content: "# About me\n\n## Preferred name\n\n\(trimmedName)",
                        enabled: true
                    )
                )
            }
        }
        let trimmedGoal = self.firstGoal.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmedGoal.isEmpty {
            let goals = try await DaemonClient.shared.listGoals().goals
            if !goals.contains(where: { $0.title.caseInsensitiveCompare(trimmedGoal) == .orderedSame }) {
                _ = try await DaemonClient.shared.createGoal(
                    GoalCreateRequest(
                        title: trimmedGoal,
                        summary: L10n.tr("setup.personalize.goal.summary"),
                        whyItMatters: nil,
                        priority: 3
                    )
                )
            }
        }
    }

    private func goBack() {
        if self.currentStep == .localSetup {
            self.isLeavingLocalSetup = true
            return
        }
        self.navigation.goBack()
    }

    private func finishGoingBack(cancelInstall: Bool) {
        guard cancelInstall else {
            self.navigation.goBack()
            return
        }
        Task {
            await self.navigation.goBack(after: { try await LocalRuntimeInstall.cancel() })
        }
    }
}
