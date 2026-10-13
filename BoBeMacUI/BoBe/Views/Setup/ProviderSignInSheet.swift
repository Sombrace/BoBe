import SwiftUI

struct ProviderSignInSheet: View {
    let providerName: String
    let onSuccess: () -> Void
    let onClose: () -> Void

    @Environment(\.theme) private var theme
    @State private var runner: ProviderLoginRunner
    @State private var response = ""
    @State private var expertMode = ExpertMode.shared

    init(
        profile: String,
        providerName: String,
        selection: EndpointSelection,
        onSuccess: @escaping () -> Void,
        onClose: @escaping () -> Void
    ) {
        self.providerName = providerName
        self.onSuccess = onSuccess
        self.onClose = onClose
        self._runner = State(initialValue: ProviderLoginRunner(profile: profile, selection: selection))
    }

    var body: some View {
        VStack(spacing: 22) {
            self.header
            self.phaseView
            self.footer
        }
        .padding(28)
        .frame(width: 460)
        .background(self.theme.colors.background)
        .task { self.runner.start() }
        .onChange(of: self.runner.state) { _, newValue in
            self.response = ""
            if case .completed = newValue { self.onSuccess() }
        }
        .interactiveDismissDisabled(!self.runner.state.isTerminal)
        .disabled(self.runner.isCanceling)
        .onDisappear {
            self.response = ""
            Task { await self.runner.cancel() }
        }
    }

    private var header: some View {
        VStack(spacing: 6) {
            Image(systemName: "person.crop.circle.badge.checkmark")
                .font(.system(size: 36, weight: .semibold))
                .foregroundStyle(self.theme.colors.primary)
            Text(L10n.tr("setup.cloud_auth.sheet.title"))
                .bobeTextStyle(.setupHeading)
                .foregroundStyle(self.theme.colors.text)
            Text(self.providerName)
                .bobeTextStyle(.settingsBody)
                .foregroundStyle(self.theme.colors.text)
            Text(L10n.tr("setup.cloud_auth.sheet.subtitle"))
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.textMuted)
                .multilineTextAlignment(.center)
        }
    }

    @ViewBuilder
    private var phaseView: some View {
        switch self.runner.state {
        case .idle, .preparing:
            VStack(spacing: 10) {
                BobeSpinner(size: 16)
                Text(L10n.tr("setup.cloud_auth.sheet.preparing"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
            }
            .frame(maxWidth: .infinity, minHeight: 160)
        case let .awaitingUser(url, code), let .polling(url, code):
            self.codeBody(url: url, code: code)
        case let .awaitingInput(message, password):
            self.inputBody(message: message, password: password)
        case .completed:
            self.terminalBody(
                icon: "checkmark.seal.fill",
                text: L10n.tr("setup.cloud_auth.sheet.success")
            )
        case let .failed(message):
            self.failedBody(message: message)
        case .canceled:
            self.terminalBody(icon: "xmark.circle", text: L10n.tr("setup.cloud_auth.sheet.canceled"))
        }
    }

    private func codeBody(url: URL, code: String) -> some View {
        VStack(spacing: 16) {
            VStack(spacing: 6) {
                Text(L10n.tr("setup.cloud_auth.sheet.step1"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                Button {
                    self.runner.openVerificationURL()
                } label: {
                    Label(L10n.tr("setup.cloud_auth.sheet.open_browser"), systemImage: "arrow.up.right.square")
                }
                .bobeButton(.primary, size: .regular)
                Text(url.absoluteString)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(self.theme.colors.textMuted)
                    .lineLimit(3)
                    .textSelection(.enabled)
            }
            if !code.isEmpty {
                VStack(spacing: 8) {
                    Text(L10n.tr("setup.cloud_auth.sheet.step2"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                    HStack(spacing: 10) {
                        Text(code)
                            .font(.system(size: 28, weight: .bold, design: .monospaced))
                            .kerning(2)
                            .minimumScaleFactor(0.7)
                            .lineLimit(1)
                            .foregroundStyle(self.theme.colors.text)
                            .padding(.horizontal, 18)
                            .padding(.vertical, 10)
                            .bobeRaised(cornerRadius: 16)
                            .textSelection(.enabled)
                        Button {
                            self.runner.copyCode()
                        } label: {
                            Label(
                                self.runner.didCopyCode
                                    ? L10n.tr("setup.cloud_auth.sheet.copied")
                                    : L10n.tr("setup.cloud_auth.sheet.copy"),
                                systemImage: self.runner.didCopyCode ? "checkmark" : "doc.on.doc"
                            )
                        }
                        .bobeButton(.secondary, size: .small)
                    }
                }
            }
            HStack(spacing: 6) {
                BobeSpinner(size: 12)
                Text(L10n.tr("setup.cloud_auth.sheet.waiting"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
            }
        }
    }

    private func inputBody(message: String, password: Bool) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(message)
                .bobeTextStyle(.settingsBody)
                .foregroundStyle(self.theme.colors.text)
                .textSelection(.enabled)
            Group {
                if password {
                    SecureField(L10n.tr("setup.cloud_auth.sheet.response"), text: self.$response)
                } else {
                    TextField(L10n.tr("setup.cloud_auth.sheet.response"), text: self.$response)
                }
            }
            .bobeField()
            .onSubmit { self.submitResponse() }
            .disabled(self.runner.isSubmitting)
            if let error = self.runner.submissionError { SettingsErrorBanner(message: error) }
            Button(L10n.tr("setup.cloud_auth.sheet.submit")) { self.submitResponse() }
                .bobeButton(.primary, size: .regular)
                .disabled(self.runner.isSubmitting)
            if self.runner.isSubmitting { BobeSpinner(size: 12) }
        }
        .frame(maxWidth: .infinity, minHeight: 160)
    }

    private func submitResponse() {
        let value = self.response
        Task {
            await self.runner.submit(value)
            if self.runner.submissionError == nil { self.response = "" }
        }
    }

    private func terminalBody(icon: String, text: String) -> some View {
        VStack(spacing: 10) {
            Image(systemName: icon)
                .font(.system(size: 36))
                .foregroundStyle(self.theme.colors.secondary)
            Text(text)
                .bobeTextStyle(.setupHeading)
                .foregroundStyle(self.theme.colors.text)
        }
        .frame(maxWidth: .infinity, minHeight: 140)
    }

    private func failedBody(message: String) -> some View {
        VStack(spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .font(.system(size: 32))
                .foregroundStyle(self.theme.colors.primary)
            Text(L10n.tr("setup.cloud_auth.sheet.failed_title"))
                .bobeTextStyle(.setupHeading)
                .foregroundStyle(self.theme.colors.text)
            Text(Self.humanize(failure: message))
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.textMuted)
                .multilineTextAlignment(.center)
            if self.expertMode.isEnabled {
                BobeDisclosure(L10n.tr("setup.cloud_auth.sheet.error_details")) {
                    Text(message)
                        .font(.system(size: 10, design: .monospaced))
                        .foregroundStyle(self.theme.colors.textMuted)
                        .textSelection(.enabled)
                        .padding(.top, 4)
                }
                .bobeTextStyle(.helper)
            }
        }
        .frame(maxWidth: .infinity, minHeight: 160)
    }

    static func humanize(failure raw: String) -> String {
        let lower = raw.lowercased()
        if lower.contains("expired") { return L10n.tr("setup.cloud_auth.sheet.error.expired") }
        if lower.contains("denied") { return L10n.tr("setup.cloud_auth.sheet.error.denied") }
        if lower.contains("slow_down") { return L10n.tr("setup.cloud_auth.sheet.error.slow_down") }
        if lower.contains("network") || lower.contains("offline") || lower.contains("timeout") {
            return L10n.tr("setup.cloud_auth.sheet.error.network")
        }
        if lower.contains("pkce") || lower.contains("device_code")
            || lower.range(of: "(^|[^0-9])400([^0-9]|$)", options: .regularExpression) != nil {
            return L10n.tr("setup.cloud_auth.sheet.error.handshake")
        }
        return L10n.tr("setup.cloud_auth.sheet.error.generic")
    }

    @ViewBuilder
    private var footer: some View {
        switch self.runner.state {
        case .idle, .preparing, .awaitingUser, .polling, .awaitingInput:
            Button(L10n.tr("setup.cloud_auth.sheet.cancel"), action: self.onClose)
                .bobeButton(.ghost, size: .small)
                .keyboardShortcut(.cancelAction)
        case .completed:
            Button(L10n.tr("setup.cloud_auth.sheet.done"), action: self.onClose)
                .bobeButton(.primary, size: .regular)
                .keyboardShortcut(.defaultAction)
        case .failed, .canceled:
            HStack(spacing: 10) {
                Button(L10n.tr("setup.cloud_auth.sheet.try_again")) { self.runner.start() }
                    .bobeButton(.primary, size: .regular)
                Button(L10n.tr("setup.cloud_auth.sheet.cancel")) { self.cancelAndClose() }
                    .bobeButton(.ghost, size: .small)
                    .keyboardShortcut(.cancelAction)
            }
        }
    }

    private func cancelAndClose() {
        Task {
            if await self.runner.cancel() { self.onClose() }
        }
    }
}
