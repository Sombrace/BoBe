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
    case saving
    case verifying

    var title: String {
        switch self {
        case .starting: L10n.tr("setup.runtime.phase.starting")
        case .checking: L10n.tr("setup.runtime.phase.checking")
        case .pairing: L10n.tr("setup.runtime.phase.pairing")
        case .saving: L10n.tr("setup.runtime.phase.saving")
        case .verifying: L10n.tr("setup.runtime.phase.verifying")
        }
    }
}

private enum RuntimeSetupOperation: Equatable {
    case editing
    case working(RuntimeSetupPhase)
    case awaitingAttach(profileID: UUID)

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
            self._operation = State(initialValue: .awaitingAttach(profileID: initialProfile.id))
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
        self.operationTask = Task { @MainActor in
            do {
                let initializationState = try await self.selectRuntime()
                self.finishOperation()
                self.onContinue(initializationState)
            } catch is CancellationError {
                self.finishOperation()
            } catch {
                if case .awaitingAttach = self.operation {
                    self.errorMessage = RuntimeIssue(error).message
                    self.onCriticalOperationChange(false)
                    return
                }
                await self.restorePreviousRuntime(
                    previousProfile: previousProfile,
                    previousCredential: previousCredential
                )
                self.errorMessage = RuntimeIssue(error).message
                self.finishOperation()
            }
        }
    }

    private func retryAttach() {
        guard case let .awaitingAttach(profileID) = self.operation,
              let profile = EndpointStore.shared.profile,
              profile.id == profileID
        else {
            self.returnToPairing()
            return
        }
        self.errorMessage = nil
        self.setOperation(.working(.verifying))
        let selection = EndpointStore.shared.selection
        self.operationTask = Task { @MainActor in
            do {
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
                self.finishOperation()
                self.onContinue(result.hello.initializationState)
            } catch is CancellationError {
                self.finishOperation()
            } catch {
                self.operation = .awaitingAttach(profileID: profileID)
                self.errorMessage = RuntimeIssue(error).message
                self.onCriticalOperationChange(false)
            }
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

    private func selectRuntime() async throws -> InitializationStateWire {
        switch self.selection {
        case .managedLocal:
            let existing = EndpointStore.shared.profile
            let hasManagedCredential = existing?.mode == .managedLocal
                && existing.flatMap(EndpointStore.shared.credential) != nil
            if !hasManagedCredential {
                try EndpointStore.shared.persist(
                    EndpointProfile.managedLocal,
                    credential: nil,
                    deletePreviousCredential: true
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
            return attach.hello.initializationState

        case .attachedLocal:
            return try await self.pairAndPersist(
                address: self.attachedURL,
                code: self.attachedCode,
                mode: .attachedLocal
            )

        case .remote:
            return try await self.pairAndPersist(
                address: self.remoteURL,
                code: self.remoteCode,
                mode: .remote
            )
        }
    }

    private func pairAndPersist(
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
        self.setOperation(.working(.saving))
        try EndpointStore.shared.persist(
            profile,
            credential: credential,
            deletePreviousCredential: false
        )
        switch mode {
        case .attachedLocal: self.attachedCode = ""
        case .remote: self.remoteCode = ""
        case .managedLocal: break
        }
        self.setOperation(.awaitingAttach(profileID: profile.id))
        self.onCriticalOperationChange(false)
        self.setOperation(.working(.verifying))
        do {
            let attach = try await EndpointAttachService.shared.attach(
                profile: profile,
                credential: credential
            )
            guard attach.hello.runtimeAuthorityId == profile.runtimeAuthorityId,
                  attach.hello.runtimeAuthorityEpoch == profile.runtimeAuthorityEpoch
            else { throw RuntimeLocationError.identityMismatch }
            return attach.hello.initializationState
        } catch {
            self.operation = .awaitingAttach(profileID: profile.id)
            throw error
        }
    }

    private func restorePreviousRuntime(
        previousProfile: EndpointProfile?,
        previousCredential: String?
    ) async {
        let failedProfile = EndpointStore.shared.profile
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
