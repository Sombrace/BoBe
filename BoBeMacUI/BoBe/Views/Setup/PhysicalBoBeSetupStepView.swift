import SwiftUI

struct PhysicalBoBeSetupStepView: View {
    let onContinue: () -> Void

    @Environment(\.theme) private var theme
    @State private var ssid = ""
    @State private var password = ""
    @State private var phase: Phase = .idle
    @State private var error: String?
    @State private var configuredIdentity: String?
    @State private var configurationTask: Task<Void, Never>?
    @FocusState private var focusedField: Field?

    private enum Field {
        case ssid
        case password
    }

    private enum Phase {
        case idle
        case configuring
        case connected
    }

    var body: some View {
        VStack(spacing: 18) {
            SetupStepHeader(title: L10n.tr("setup.physical.title"), message: L10n.tr("setup.physical.body"))
                .padding(.top, 8)

            VStack(alignment: .leading, spacing: 10) {
                Text(L10n.tr("setup.physical.network.label"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                BobeTextField(
                    placeholder: L10n.tr("setup.physical.network.placeholder"),
                    text: self.$ssid,
                    width: 340
                )
                .focused(self.$focusedField, equals: .ssid)
                .accessibilityIdentifier("setup.physical.ssid")
                Text(L10n.tr("setup.physical.password.label"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
                BobeSecureField(
                    placeholder: L10n.tr("setup.physical.password.placeholder"),
                    text: self.$password,
                    width: 340
                )
                .focused(self.$focusedField, equals: .password)
                .accessibilityIdentifier("setup.physical.password")
                Text(L10n.tr("setup.physical.password.help"))
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.theme.colors.textMuted)
            }
            .disabled(self.phase == .configuring)

            if self.phase == .configuring {
                HStack(spacing: 10) {
                    BobeSpinner(size: 14)
                    Text(L10n.tr("setup.physical.connecting"))
                        .bobeTextStyle(.setupBody)
                        .foregroundStyle(self.theme.colors.textMuted)
                }
            } else if self.phase == .connected {
                Label(
                    L10n.tr(
                        "setup.physical.connected_format",
                        String(self.configuredIdentity?.suffix(6) ?? ""),
                        self.ssid
                    ),
                    systemImage: "checkmark.circle.fill"
                )
                    .bobeTextStyle(.setupBody)
                    .foregroundStyle(self.theme.colors.secondary)
            } else if let error {
                Label(error, systemImage: "exclamationmark.triangle.fill")
                    .bobeTextStyle(.setupBody)
                    .foregroundStyle(self.theme.colors.destructive)
                    .multilineTextAlignment(.center)
            }

            Spacer()

            HStack(spacing: 12) {
                if self.phase == .connected {
                    Button(L10n.tr("setup.physical.continue"), action: self.onContinue)
                        .bobeButton(.primary)
                        .keyboardShortcut(.defaultAction)
                } else {
                    Button(L10n.tr("setup.physical.not_now"), action: self.onContinue)
                        .bobeButton(.ghost)
                        .disabled(self.phase == .configuring)
                        .accessibilityIdentifier("setup.physical.skip")
                    Button(L10n.tr("setup.physical.configure")) {
                        self.configurationTask = Task {
                            await self.configure()
                        }
                    }
                    .bobeButton(.primary)
                    .disabled(
                        self.phase == .configuring ||
                            self.ssid.isEmpty ||
                            (!self.password.isEmpty && self.password.utf8.count < 8)
                    )
                    .keyboardShortcut(.defaultAction)
                    .accessibilityIdentifier("setup.physical.configure")
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("setup.physical")
        .onAppear { self.focusedField = .ssid }
        .onDisappear {
            self.configurationTask?.cancel()
            self.configurationTask = nil
            self.password = ""
        }
    }

    private func configure() async {
        self.phase = .configuring
        self.error = nil
        do {
            self.configuredIdentity = try await WB12OnboardingService.shared.configure(
                ssid: self.ssid,
                password: self.password
            )
            self.password = ""
            self.phase = .connected
            self.configurationTask = nil
        } catch is CancellationError {
            self.password = ""
            self.phase = .idle
            self.configurationTask = nil
        } catch {
            self.phase = .idle
            self.configurationTask = nil
            self.error = (error as? WB12OnboardingError)?.localizedDescription
                ?? L10n.tr("setup.physical.disconnected")
        }
    }
}
