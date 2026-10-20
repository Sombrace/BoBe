import CoreImage.CIFilterBuiltins
import SwiftUI

struct DevicesPanel: View {
    @Environment(\.theme) private var theme
    @State private var endpoint = EndpointStore.shared.selection
    @State private var devices: [DeviceWire]?
    @State private var error: String?
    @State private var pairing = false
    @State private var removing: DeviceWire?
    @State private var access: Bool?
    @State private var accessTarget: Bool?
    @State private var changingAccess = false

    private var activeDevices: [DeviceWire] {
        (self.devices ?? []).filter { $0.revokedAt == nil }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 24) {
                HStack(alignment: .top, spacing: 16) {
                    BobeIconTile(systemName: "iphone", tint: SettingsCategory.devices.tint, size: 44)
                    VStack(alignment: .leading, spacing: 8) {
                        Text(L10n.tr("settings.devices.phone.title"))
                            .bobeTextStyle(.heading)
                            .foregroundStyle(self.theme.colors.text)
                        Button(L10n.tr("settings.devices.phone.pair")) { self.pairing = true }
                            .bobeButton(.primary, size: .regular)
                            .accessibilityIdentifier("settings.devices.pair_phone")
                            .padding(.top, 4)
                    }
                }

                if self.endpoint.endpoint.canRestartRuntime, let access = self.access {
                    SettingsRow(label: L10n.tr("settings.devices.access")) {
                        if self.changingAccess {
                            BobeSpinner(size: 12)
                        }
                        BobeToggle(
                            isOn: Binding(get: { access }, set: { self.accessTarget = $0 }),
                            accessibilityLabel: L10n.tr("settings.devices.access")
                        )
                        .disabled(self.changingAccess)
                        .accessibilityIdentifier("settings.devices.access")
                    }
                }

                BobeHairline()

                VStack(alignment: .leading, spacing: 10) {
                    Text(L10n.tr("settings.devices.list.title"))
                        .bobeTextStyle(.setupHeading)
                        .foregroundStyle(self.theme.colors.text)
                    if self.devices == nil, self.error == nil {
                        BobeSpinner(size: 12)
                    } else if self.activeDevices.isEmpty, self.error == nil {
                        Text(L10n.tr("settings.devices.list.empty"))
                            .bobeTextStyle(.helper)
                            .foregroundStyle(self.theme.colors.textMuted)
                    }
                    ForEach(self.activeDevices) { device in
                        DeviceRow(device: device) { self.removing = device }
                    }
                    if let error {
                        SettingsErrorBanner(message: error)
                    }
                }
            }
            .padding(24)
        }
        .sheet(
            isPresented: self.$pairing,
            onDismiss: { Task { await self.refresh() } },
            content: {
                PairPhoneSheet(endpoint: self.endpoint)
                    .environment(\.theme, self.theme)
            }
        )
        .confirmationDialog(
            L10n.tr("settings.devices.remove.title_format", self.removing?.label ?? ""),
            isPresented: Binding(
                get: { self.removing != nil },
                set: { if !$0 { self.removing = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button(L10n.tr("settings.devices.remove"), role: .destructive) {
                if let device = self.removing {
                    Task { await self.remove(device) }
                }
            }
        } message: {
            Text(L10n.tr("settings.devices.remove.message"))
        }
        .confirmationDialog(
            L10n.tr(self.accessTarget == true ? "settings.devices.access.on.title" : "settings.devices.access.off.title"),
            isPresented: Binding(
                get: { self.accessTarget != nil },
                set: { if !$0 { self.accessTarget = nil } }
            ),
            titleVisibility: .visible
        ) {
            if let target = self.accessTarget {
                Button(
                    L10n.tr(target ? "settings.devices.access.on.confirm" : "settings.devices.access.off.confirm"),
                    role: target ? nil : .destructive
                ) {
                    Task { await self.setAccess(target) }
                }
            }
        } message: {
            Text(L10n.tr(
                self.accessTarget == true ? "settings.devices.access.on.message" : "settings.devices.access.off.message"
            ))
        }
        .task(id: self.endpoint) {
            await self.refresh()
        }
        .onReceive(NotificationCenter.default.publisher(for: .bobeEndpointIdentityChanged)) { _ in
            self.endpoint = EndpointStore.shared.selection
            self.devices = nil
            self.error = nil
        }
    }

    private func refresh() async {
        let endpoint = EndpointStore.shared.selection
        do {
            let devices = try await DaemonClient(endpoint: endpoint.endpoint).devices()
            guard EndpointStore.shared.selection == endpoint else { return }
            self.devices = devices
            self.error = nil
        } catch {
            guard EndpointStore.shared.selection == endpoint else { return }
            self.error = L10n.tr("settings.devices.list.failed")
        }
        guard endpoint.endpoint.canRestartRuntime,
              let attach = try? await EndpointAttachService.shared.attachConfiguredEndpoint(selection: endpoint),
              EndpointStore.shared.selection == endpoint
        else { return }
        self.access = attach.hello.deploymentMode == "networked"
    }

    private func setAccess(_ reachable: Bool) async {
        let endpoint = EndpointStore.shared.selection
        self.accessTarget = nil
        self.changingAccess = true
        defer { self.changingAccess = false }
        do {
            try await NetworkReach.set(reachable, endpoint: endpoint)
            self.error = nil
        } catch {
            guard EndpointStore.shared.selection == endpoint else { return }
            self.error = L10n.tr("settings.devices.access.failed")
        }
        await self.refresh()
    }

    private func remove(_ device: DeviceWire) async {
        let endpoint = EndpointStore.shared.selection
        do {
            try await DaemonClient(endpoint: endpoint.endpoint).removeDevice(device.id)
            await self.refresh()
        } catch {
            guard EndpointStore.shared.selection == endpoint else { return }
            self.error = L10n.tr("settings.devices.remove.failed")
        }
        self.removing = nil
    }
}

private struct DeviceRow: View {
    let device: DeviceWire
    let onRemove: () -> Void
    @Environment(\.theme) private var theme

    private var isThisMac: Bool {
        self.device.id.caseInsensitiveCompare(SurfaceIdentity.current().uuidString) == .orderedSame
    }

    private var symbol: String {
        switch self.device.kind {
        case "mac_app": "laptopcomputer"
        case "physical_body": "circle.circle.fill"
        default: "iphone"
        }
    }

    private var kindLabel: String {
        switch self.device.kind {
        case "mac_app": L10n.tr("settings.devices.kind.mac")
        case "physical_body": L10n.tr("settings.devices.kind.body")
        default: L10n.tr("settings.devices.kind.phone")
        }
    }

    var body: some View {
        HStack(spacing: 12) {
            BobeIconTile(systemName: self.symbol, tint: self.theme.colors.tertiary, size: 30)
            VStack(alignment: .leading, spacing: 2) {
                Text(self.device.label)
                    .bobeTextStyle(.rowTitle)
                    .foregroundStyle(self.theme.colors.text)
                Text(self.isThisMac ? L10n.tr("settings.devices.this_mac") : self.kindLabel)
                    .bobeTextStyle(.rowMeta)
                    .foregroundStyle(self.theme.colors.textMuted)
            }
            Spacer(minLength: 8)
            if !self.isThisMac {
                Button(L10n.tr("settings.devices.remove"), action: self.onRemove)
                    .bobeButton(.secondary, size: .small)
            }
        }
        .padding(12)
        .bobeRaised(cornerRadius: BobeRadius.card)
    }
}

private struct PairPhoneSheet: View {
    @State var endpoint: EndpointSelection
    @Environment(\.dismiss) private var dismiss
    @Environment(\.theme) private var theme
    @State private var invitation: PhoneInvitationWire?
    @State private var problem: String?
    @State private var notNetworked = false
    @State private var asksAccess = false
    @State private var remaining = 0
    @State private var loading = false

    private var expired: Bool {
        self.invitation != nil && self.remaining <= 0
    }

    private var placeholderSymbol: String {
        if self.asksAccess { return "iphone.radiowaves.left.and.right" }
        return self.expired ? "clock.badge.xmark" : "qrcode"
    }

    var body: some View {
        VStack(spacing: 16) {
            SetupStepHeader(
                title: L10n.tr(self.asksAccess ? "settings.devices.access.on.title" : "settings.devices.pair.title"),
                message: L10n.tr(self.asksAccess ? "settings.devices.access.on.message" : "settings.devices.pair.body"),
                alignment: .center
            )
            ZStack {
                RoundedRectangle(cornerRadius: BobeRadius.card, style: .continuous)
                    .fill(Color.white)
                if let invitation, !self.expired, let image = PairingQR.image(for: invitation.link) {
                    Image(decorative: image, scale: 1)
                        .interpolation(.none)
                        .resizable()
                        .aspectRatio(1, contentMode: .fit)
                        .padding(18)
                        .accessibilityLabel(L10n.tr("settings.devices.pair.qr"))
                } else if self.loading {
                    BobeSpinner(size: 18)
                } else {
                    Image(systemName: self.placeholderSymbol)
                        .font(.system(size: 40))
                        .foregroundStyle(Color.gray.opacity(0.6))
                }
            }
            .frame(width: 236, height: 236)
            .bobeRaised(cornerRadius: BobeRadius.card)

            if let invitation {
                VStack(spacing: 4) {
                    Text(L10n.tr("settings.devices.pair.code_label"))
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                    Text(Self.spaced(invitation.code))
                        .font(.system(size: 30, weight: .semibold, design: .rounded))
                        .monospacedDigit()
                        .foregroundStyle(self.expired ? self.theme.colors.textMuted : self.theme.colors.text)
                        .textSelection(.enabled)
                    Text(
                        self.expired
                            ? L10n.tr("settings.devices.pair.expired")
                            : L10n.tr("settings.devices.pair.expires_format", Self.clock(self.remaining))
                    )
                    .bobeTextStyle(.helper)
                    .foregroundStyle(self.expired ? self.theme.colors.warning : self.theme.colors.textMuted)
                    if let origin = invitation.origins.first {
                        Text(L10n.tr("settings.devices.pair.address_format", origin))
                            .bobeTextStyle(.helper)
                            .foregroundStyle(self.theme.colors.textMuted)
                            .textSelection(.enabled)
                    }
                }
            }

            if let problem {
                SettingsErrorBanner(message: problem)
            }

            HStack {
                if self.asksAccess {
                    Button(L10n.tr("settings.devices.access.cancel")) { self.dismiss() }
                        .bobeButton(.secondary, size: .regular)
                        .disabled(self.loading)
                    Spacer()
                    Button(L10n.tr("settings.devices.access.on.confirm")) {
                        Task { await self.allowAccess() }
                    }
                    .bobeButton(.primary, size: .regular)
                    .keyboardShortcut(.defaultAction)
                    .disabled(self.loading)
                    .accessibilityIdentifier("settings.devices.access.allow")
                } else {
                    Button(L10n.tr("settings.devices.pair.new_code")) {
                        Task { await self.create() }
                    }
                    .bobeButton(.secondary, size: .regular)
                    .disabled(self.loading || self.notNetworked)
                    Spacer()
                    Button(L10n.tr("settings.devices.pair.done")) { self.dismiss() }
                        .bobeButton(.primary, size: .regular)
                        .keyboardShortcut(.defaultAction)
                }
            }
            .padding(.top, 4)
        }
        .padding(28)
        .frame(width: 440)
        .background(self.theme.colors.background)
        .task { await self.create() }
        .task(id: self.invitation?.code) {
            while self.remaining > 0, !Task.isCancelled {
                try? await Task.sleep(for: .seconds(1))
                self.remaining -= 1
            }
        }
    }

    private func create() async {
        self.loading = true
        self.problem = nil
        defer { self.loading = false }
        do {
            let label = L10n.tr("settings.devices.pair.default_label")
            let invitation = try await DaemonClient(endpoint: self.endpoint.endpoint).createPhoneInvitation(label: label)
            self.invitation = invitation
            self.remaining = invitation.expiresInSeconds
            self.notNetworked = false
        } catch let DaemonError.httpError(_, _, code) where code == "NOT_NETWORKED" {
            self.invitation = nil
            self.notNetworked = true
            if self.endpoint.endpoint.canRestartRuntime {
                self.asksAccess = true
            } else {
                self.problem = L10n.tr("settings.devices.pair.not_networked")
            }
        } catch {
            self.invitation = nil
            self.problem = L10n.tr("settings.devices.pair.failed")
        }
    }

    private func allowAccess() async {
        self.loading = true
        self.problem = nil
        do {
            try await NetworkReach.set(true, endpoint: self.endpoint)
        } catch {
            self.loading = false
            self.problem = L10n.tr("settings.devices.access.failed")
            return
        }
        self.endpoint = EndpointStore.shared.selection
        self.asksAccess = false
        await self.create()
    }

    private static func spaced(_ code: String) -> String {
        guard code.count == 6 else { return code }
        return "\(code.prefix(3)) \(code.suffix(3))"
    }

    private static func clock(_ seconds: Int) -> String {
        Duration.seconds(max(seconds, 0)).formatted(.time(pattern: .minuteSecond))
    }
}

enum PairingQR {
    static func image(for text: String) -> CGImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 12, y: 12)) else {
            return nil
        }
        return CIContext().createCGImage(output, from: output.extent)
    }
}
