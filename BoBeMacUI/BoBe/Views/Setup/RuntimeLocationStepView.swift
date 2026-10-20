import SwiftUI

private enum RuntimeLocationChoice: String, CaseIterable {
    case managedLocal
    case attachedLocal
    case remote
}

private enum RuntimeSetupPhase: Equatable {
    case starting
    case checking
    case pairing
    case verifying

    var title: String {
        switch self {
        case .starting: L10n.tr("setup.runtime.phase.starting")
        case .checking: L10n.tr("setup.runtime.phase.checking")
        case .pairing: L10n.tr("setup.runtime.phase.pairing")
        case .verifying: L10n.tr("setup.runtime.phase.verifying")
        }
    }
}

nonisolated struct PairedRuntime: Equatable, Sendable {
    let profile: EndpointProfile
    let credential: String
}

enum RuntimePairing {
    static func commit(
        _ paired: PairedRuntime,
        store: EndpointStore = .shared,
        attach: @Sendable (EndpointProfile, String) async throws -> AttachResult = { profile, credential in
            try await EndpointAttachService.shared.attach(profile: profile, credential: credential)
        }
    ) async throws -> InitializationStateWire {
        let result = try await attach(paired.profile, paired.credential)
        guard result.hello.runtimeAuthorityId == paired.profile.runtimeAuthorityId,
              result.hello.runtimeAuthorityEpoch == paired.profile.runtimeAuthorityEpoch
        else { throw RuntimeLocationError.identityMismatch }
        try Task.checkCancellation()
        try store.persist(paired.profile, credential: paired.credential, deletePreviousCredential: true)
        return result.hello.initializationState
    }
}

private enum RuntimeAttachTarget: Equatable {
    case stored(profileID: UUID)
    case paired(PairedRuntime)
}

private enum RuntimeSetupOperation: Equatable {
    case editing
    case working(RuntimeSetupPhase)
    case awaitingAttach(RuntimeAttachTarget)

    var isWorking: Bool {
        if case .working = self { return true }
        return false
    }
}

private struct RuntimeConnectionFieldConfig {
    let addressPlaceholder: String
    let help: String
    let mode: EndpointProfile.Mode
}

struct RuntimeLocationStepView: View {
    let onContinue: (InitializationStateWire) -> Void
    var onCriticalOperationChange: (Bool) -> Void

    @Environment(\.theme) private var theme
    @State private var selection: RuntimeLocationChoice
    @State private var remoteURL: String
    @State private var remoteCode = ""
    @State private var attachedURL: String
    @State private var attachedCode = ""
    @State private var localDaemonDetected = false
    @State private var operation: RuntimeSetupOperation = .editing
    @State private var operationTask: Task<Void, Never>?
    @State private var errorMessage: String?
    @State private var attachedAddressFocused = false
    @State private var attachedCodeFocused = false
    @State private var remoteAddressFocused = false
    @State private var remoteCodeFocused = false

    init(
        onContinue: @escaping (InitializationStateWire) -> Void,
        onCriticalOperationChange: @escaping (Bool) -> Void = { _ in },
        initialProfile: EndpointProfile? = nil,
        initialIssue: RuntimeIssue? = nil
    ) {
        self.onContinue = onContinue
        self.onCriticalOperationChange = onCriticalOperationChange
        let choice: RuntimeLocationChoice = switch initialProfile?.mode {
        case .attachedLocal: .attachedLocal
        case .remote: .remote
        case .managedLocal, nil: .managedLocal
        }
        self._selection = State(initialValue: choice)
        self._remoteURL = State(
            initialValue: initialProfile?.mode == .remote
                ? initialProfile?.baseURL.absoluteString ?? ""
                : ""
        )
        self._attachedURL = State(
            initialValue: initialProfile?.mode == .attachedLocal
                ? initialProfile?.baseURL.absoluteString ?? ""
                : "http://127.0.0.1:\(DaemonConfig.managedPort)"
        )
        self._errorMessage = State(initialValue: initialIssue?.message)
        if let initialProfile,
           initialIssue?.primaryAction == .retryConnection {
            self._operation = State(initialValue: .awaitingAttach(.stored(profileID: initialProfile.id)))
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(spacing: 22) {
                    SetupStepHeader(title: L10n.tr("setup.runtime.title"))
                        .padding(.top, 8)

                    VStack(spacing: 10) {
                        self.choiceCard(
                            .managedLocal,
                            icon: "laptopcomputer",
                            title: L10n.tr("setup.runtime.local.title"),
                            subtitle: L10n.tr("setup.runtime.local.subtitle")
                        )
                        self.choiceCard(
                            .attachedLocal,
                            icon: "bolt.horizontal.fill",
                            title: L10n.tr("setup.runtime.attached.title"),
                            subtitle: self.localDaemonDetected
                                ? L10n.tr("setup.runtime.attached.subtitle")
                                : L10n.tr("setup.runtime.attached.manual_subtitle")
                        )
                        self.choiceCard(
                            .remote,
                            icon: "externaldrive.connected.to.line.below",
                            title: L10n.tr("setup.runtime.remote.title"),
                            subtitle: L10n.tr("setup.runtime.remote.subtitle")
                        )
                    }

                    if self.selection == .attachedLocal {
                        self.connectionFields(
                            address: self.$attachedURL,
                            code: self.$attachedCode,
                            addressFocused: self.$attachedAddressFocused,
                            codeFocused: self.$attachedCodeFocused,
                            config: RuntimeConnectionFieldConfig(
                                addressPlaceholder: L10n.tr("setup.runtime.attached.url.placeholder"),
                                help: L10n.tr("setup.runtime.attached.help"),
                                mode: .attachedLocal
                            )
                        )
                        .transition(.opacity.combined(with: .move(edge: .top)))
                    }

                    if self.selection == .remote {
                        self.connectionFields(
                            address: self.$remoteURL,
                            code: self.$remoteCode,
                            addressFocused: self.$remoteAddressFocused,
                            codeFocused: self.$remoteCodeFocused,
                            config: RuntimeConnectionFieldConfig(
                                addressPlaceholder: L10n.tr("setup.runtime.remote.url.placeholder"),
                                help: L10n.tr("setup.runtime.remote.help"),
                                mode: .remote
                            )
                        )
                        .transition(.opacity.combined(with: .move(edge: .top)))
                    }

                    if let errorMessage {
                        Label(errorMessage, systemImage: "exclamationmark.triangle.fill")
                            .bobeTextStyle(.helper)
                            .foregroundStyle(self.theme.colors.warning)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .accessibilityIdentifier("setup.runtime.error")
                    }
                }
                .padding(.top, 4)
                .padding(.bottom, 18)
            }

            BobeHairline()
                .overlay(self.theme.colors.border)

            VStack(spacing: 8) {
                Button(action: self.primaryAction) {
                    HStack(spacing: 8) {
                        if self.operation.isWorking {
                            BobeSpinner(size: 14)
                        }
                        Text(self.primaryActionTitle)
                    }
                }
                .bobeButton(.primary)
                .disabled(!self.canPerformPrimaryAction)
                .keyboardShortcut(.defaultAction)
                .accessibilityIdentifier("setup.runtime.continue")

                if case .awaitingAttach = self.operation {
                    Button(L10n.tr("setup.runtime.pair_again"), action: self.returnToPairing)
                        .buttonStyle(.plain)
                        .foregroundStyle(self.theme.colors.primary)
                        .accessibilityIdentifier("setup.runtime.pair-again")
                }
            }
            .padding(.top, 14)
        }
        .task { await self.probeAttachedLocal() }
        .onDisappear {
            self.operationTask?.cancel()
            self.operationTask = nil
            self.onCriticalOperationChange(false)
        }
    }

    private var primaryActionTitle: String {
        switch self.operation {
        case let .working(phase): phase.title
        case .awaitingAttach: L10n.tr("setup.runtime.retry_connection")
        case .editing: L10n.tr("setup.runtime.continue")
        }
    }

    private var canPerformPrimaryAction: Bool {
        switch self.operation {
        case .working:
            false
        case .awaitingAttach:
            true
        case .editing:
            switch self.selection {
            case .managedLocal:
                true
            case .attachedLocal:
                Self.isValidForm(
                    address: self.attachedURL,
                    code: self.attachedCode,
                    mode: .attachedLocal
                )
            case .remote:
                Self.isValidForm(
                    address: self.remoteURL,
                    code: self.remoteCode,
                    mode: .remote
                )
            }
        }
    }

    static func normalizedPairingCode(_ value: String) -> String {
        String(value.unicodeScalars.compactMap { scalar -> Character? in
            guard scalar.value >= 48, scalar.value <= 57 else { return nil }
            return Character(String(scalar))
        }.prefix(6))
    }

    static func isValidForm(address: String, code: String, mode: EndpointProfile.Mode) -> Bool {
        normalizedPairingCode(code).count == 6
            && DevicePairingService.isValidAddress(address, expectedMode: mode)
    }

    @ViewBuilder
    private func connectionFields(
        address: Binding<String>,
        code: Binding<String>,
        addressFocused: Binding<Bool>,
        codeFocused: Binding<Bool>,
        config: RuntimeConnectionFieldConfig
    ) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(L10n.tr("setup.runtime.address.label"))
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.text)
            BobeTextField(
                placeholder: config.addressPlaceholder,
                text: address,
                focused: addressFocused,
                onSubmit: { codeFocused.wrappedValue = true }
            )
            .disabled(self.operation.isWorking)
            .accessibilityIdentifier("setup.runtime.address")

            Text(L10n.tr("setup.runtime.code.label"))
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.text)
            BobeTextField(
                placeholder: L10n.tr("setup.runtime.remote.code.placeholder"),
                text: code,
                alignment: .center,
                focused: codeFocused,
                onSubmit: {
                    if Self.isValidForm(
                        address: address.wrappedValue,
                        code: code.wrappedValue,
                        mode: config.mode
                    ) {
                        self.commitSelection()
                    }
                }
            )
            .disabled(self.operation.isWorking)
            .accessibilityIdentifier("setup.runtime.code")
            .onChange(of: code.wrappedValue) { _, value in
                let normalized = Self.normalizedPairingCode(value)
                if normalized != value {
                    code.wrappedValue = normalized
                }
            }

            Text(config.help)
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.textMuted)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func choiceCard(
        _ choice: RuntimeLocationChoice,
        icon: String,
        title: String,
        subtitle: String
    ) -> some View {
        SetupChoiceCard(
            icon: icon,
            tint: SettingsCategory.runtime.tint,
            title: title,
            subtitle: subtitle,
            isSelected: self.selection == choice,
            action: {
                withAnimation(BobeMotion.standard) {
                    self.selection = choice
                    self.errorMessage = nil
                    self.operation = .editing
                }
                self.focusFirstField(for: choice)
            }
        )
        .disabled(self.operation.isWorking)
        .accessibilityIdentifier("setup.runtime.\(choice.rawValue)")
    }

    private func focusFirstField(for choice: RuntimeLocationChoice) {
        self.attachedAddressFocused = choice == .attachedLocal
        self.attachedCodeFocused = false
        self.remoteAddressFocused = choice == .remote
        self.remoteCodeFocused = false
    }

    private func primaryAction() {
        if case .awaitingAttach = self.operation {
            self.retryAttach()
        } else {
            self.commitSelection()
        }
    }

    private func commitSelection() {
        guard self.canPerformPrimaryAction else { return }
        self.errorMessage = nil
        self.setOperation(.working(self.selection == .managedLocal ? .starting : .checking))
        let previousProfile = EndpointStore.shared.profile
        let previousCredential = previousProfile.flatMap(EndpointStore.shared.credential)
        self.operationTask = Task {
            do {
                let initializationState = try await self.selectRuntime(replacing: previousProfile)
                self.finishOperation()
                self.onContinue(initializationState)
            } catch {
                await self.restorePreviousRuntime(
                    previousProfile: previousProfile,
                    previousCredential: previousCredential
                )
                if error is CancellationError {
                    self.finishOperation()
                    return
                }
                self.errorMessage = RuntimeIssue(error).message
                if case .awaitingAttach = self.operation {
                    self.onCriticalOperationChange(false)
                } else {
                    self.finishOperation()
                }
            }
        }
    }

    private func retryAttach() {
        guard case let .awaitingAttach(target) = self.operation else { return }
        if case let .stored(profileID) = target, EndpointStore.shared.profile?.id != profileID {
            self.returnToPairing()
            return
        }
        self.errorMessage = nil
        self.setOperation(.working(.verifying))
        self.operationTask = Task {
            do {
                let initializationState = try await self.attach(target)
                self.finishOperation()
                self.onContinue(initializationState)
            } catch is CancellationError {
                self.finishOperation()
            } catch {
                self.operation = .awaitingAttach(target)
                self.errorMessage = RuntimeIssue(error).message
                self.onCriticalOperationChange(false)
            }
        }
    }

    private func attach(_ target: RuntimeAttachTarget) async throws -> InitializationStateWire {
        switch target {
        case let .paired(paired):
            return try await RuntimePairing.commit(paired)
        case let .stored(profileID):
            let selection = EndpointStore.shared.selection
            guard let profile = EndpointStore.shared.profile(for: selection), profile.id == profileID else {
                throw CancellationError()
            }
            let result = try await EndpointAttachService.shared.attach(
                profile: profile,
                credential: selection.endpoint.bearerToken
            )
            try EndpointStore.shared.updateNegotiatedIdentity(
                profileID: profile.id,
                authorityID: result.hello.runtimeAuthorityId,
                authorityEpoch: result.hello.runtimeAuthorityEpoch,
                expectedSelection: selection
            )
            return result.hello.initializationState
        }
    }

    private func returnToPairing() {
        self.operationTask?.cancel()
        self.operationTask = nil
        self.operation = .editing
        self.errorMessage = nil
        self.onCriticalOperationChange(false)
        switch self.selection {
        case .attachedLocal: self.attachedCodeFocused = true
        case .remote: self.remoteCodeFocused = true
        case .managedLocal: break
        }
    }

    private func selectRuntime(replacing previousProfile: EndpointProfile?) async throws -> InitializationStateWire {
        switch self.selection {
        case .managedLocal:
            let existing = EndpointStore.shared.profile
            let hasManagedCredential = existing?.mode == .managedLocal
                && existing.flatMap(EndpointStore.shared.credential) != nil
            if !hasManagedCredential {
                try EndpointStore.shared.persist(
                    EndpointProfile.managedLocal,
                    credential: nil,
                    deletePreviousCredential: false
                )
            }
            try await BackendService.shared.start()
            guard let enrolledProfile = EndpointStore.shared.profile,
                  enrolledProfile.mode == .managedLocal,
                  let credential = EndpointStore.shared.credential(for: enrolledProfile)
            else { throw RuntimeLocationError.unreachable }
            self.setOperation(.working(.verifying))
            let attach = try await EndpointAttachService.shared.attach(
                profile: enrolledProfile,
                credential: credential
            )
            if let previousReference = previousProfile?.credentialReference,
               previousReference != enrolledProfile.credentialReference {
                EndpointStore.shared.deleteCredential(reference: previousReference)
            }
            return attach.hello.initializationState

        case .attachedLocal:
            return try await self.pairAndVerify(
                address: self.attachedURL,
                code: self.attachedCode,
                mode: .attachedLocal
            )

        case .remote:
            return try await self.pairAndVerify(
                address: self.remoteURL,
                code: self.remoteCode,
                mode: .remote
            )
        }
    }

    private func pairAndVerify(
        address: String,
        code: String,
        mode: EndpointProfile.Mode
    ) async throws -> InitializationStateWire {
        self.setOperation(.working(.pairing))
        let (profile, credential) = try await DevicePairingService.shared.pair(
            address: address,
            code: Self.normalizedPairingCode(code),
            expectedMode: mode
        )
        switch mode {
        case .attachedLocal: self.attachedCode = ""
        case .remote: self.remoteCode = ""
        case .managedLocal: break
        }
        let paired = PairedRuntime(profile: profile, credential: credential)
        self.setOperation(.working(.verifying))
        do {
            return try await RuntimePairing.commit(paired)
        } catch {
            self.operation = .awaitingAttach(.paired(paired))
            throw error
        }
    }

    private func restorePreviousRuntime(
        previousProfile: EndpointProfile?,
        previousCredential: String?
    ) async {
        let failedProfile = EndpointStore.shared.profile
        guard failedProfile != previousProfile else { return }
        if failedProfile?.mode == .managedLocal,
           previousProfile?.mode != .managedLocal {
            await BackendService.shared.stop()
        }
        do {
            if let previousProfile {
                try EndpointStore.shared.persist(
                    previousProfile,
                    credential: previousCredential,
                    deletePreviousCredential: true
                )
            } else {
                EndpointStore.shared.clear()
            }
        } catch {
            EndpointStore.shared.clear()
        }
    }

    private func setOperation(_ operation: RuntimeSetupOperation) {
        self.operation = operation
        self.onCriticalOperationChange(operation.isWorking)
    }

    private func finishOperation() {
        self.operationTask = nil
        self.operation = .editing
        self.onCriticalOperationChange(false)
    }

    private func probeAttachedLocal() async {
        let endpoint = DaemonEndpoint.attachedLocal(
            baseURL: URL(string: "http://127.0.0.1:\(DaemonConfig.managedPort)")
                ?? URL(fileURLWithPath: "/"),
            bearerToken: nil
        )
        self.localDaemonDetected = (try? await self.readiness(for: endpoint)) != nil
    }

    private func readiness(for endpoint: DaemonEndpoint) async throws -> ReadinessResponse {
        var request = URLRequest(url: endpoint.baseURL.appendingPathComponent("readyz"))
        endpoint.authorize(&request)
        request.timeoutInterval = 2
        let session = DaemonTransportPolicy.session(
            for: endpoint,
            purpose: .readinessProbe
        )
        let (data, response) = try await session.data(for: request)
        guard let http = response as? HTTPURLResponse, (200 ... 299).contains(http.statusCode) else {
            throw RuntimeLocationError.unreachable
        }
        return try JSONDecoder().decode(ReadinessResponse.self, from: data)
    }
}

enum RuntimeLocationError: LocalizedError {
    case identityMismatch
    case unreachable

    var errorDescription: String? {
        switch self {
        case .identityMismatch: L10n.tr("setup.runtime.error.identity_mismatch")
        case .unreachable: L10n.tr("setup.runtime.error.unreachable")
        }
    }
}
