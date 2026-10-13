import SwiftUI
import UIKit

struct FindStep: View {
    enum Field: Hashable {
        case address
        case code
    }

    @Bindable var model: PhoneModel
    @State private var typing = !CodeScanner.isAvailable
    @State private var address = ""
    @State private var code = ""
    @State private var camera = PhonePermissions.camera()
    @State private var unknownCode = false
    @State private var showsHelp = false
    @FocusState private var focus: Field?
    @Environment(\.theme) private var theme
    @Environment(\.openURL) private var openURL

    private var invite: PairingInvite? {
        PairingInvite(address: self.address, code: self.code)
    }

    var body: some View {
        VStack(spacing: 14) {
            OnboardingCopy(
                title: L10n.tr("phone.find.title"),
                message: L10n.tr(self.typing ? "phone.find.body_typed" : "phone.find.body")
            )
            .padding(.top, 6)
            Group {
                switch self.model.pairing {
                case .working:
                    PairingStatusCard(text: L10n.tr("phone.find.working"), done: false)
                case .paired:
                    PairingStatusCard(text: L10n.tr("phone.find.paired"), done: true)
                case .idle, .failed:
                    if self.typing {
                        self.manualForm
                    } else {
                        self.scanner
                    }
                }
            }
            .transition(.bobeRise)
            if case let .failed(message) = self.model.pairing {
                self.notice(message)
            } else if self.unknownCode {
                self.notice(L10n.tr("phone.find.unknown_code"))
            }
            Spacer(minLength: 0)
            HStack {
                Button(L10n.tr("phone.common.back")) { self.model.returnToWelcome() }
                    .buttonStyle(.phoneQuiet)
                Spacer()
                Button(L10n.tr("phone.find.where")) { self.showsHelp = true }
                    .buttonStyle(.phoneQuiet)
            }
            .disabled(self.model.pairing == .working)
        }
        .animation(BobeMotion.standard, value: self.typing)
        .animation(BobeMotion.standard, value: self.model.pairing)
        .sheet(isPresented: self.$showsHelp) {
            WhereIsTheCodeSheet()
                .fittedSheet()
        }
    }

    private var scanner: some View {
        VStack(spacing: 12) {
            ZStack {
                RoundedRectangle(cornerRadius: BobeRadius.sheet, style: .continuous)
                    .fill(self.theme.well)
                switch self.camera {
                case .granted where CodeScanner.isAvailable:
                    CodeScanner { self.scanned($0) }
                    ScanBrackets()
                case .granted:
                    Image(systemName: "camera")
                        .font(.system(size: 34, weight: .medium))
                        .foregroundStyle(self.theme.colors.textMuted)
                case .unknown:
                    VStack(spacing: 14) {
                        Image(systemName: "camera.viewfinder")
                            .font(.system(size: 34, weight: .medium))
                            .foregroundStyle(self.theme.colors.textMuted)
                        Button(L10n.tr("phone.find.camera.allow")) {
                            Task {
                                self.camera = await PhonePermissions.requestCamera() ? .granted : .denied
                            }
                        }
                        .buttonStyle(.phoneSecondary)
                        .fixedSize()
                    }
                case .denied:
                    VStack(spacing: 14) {
                        self.cameraMessage(symbol: "camera.fill", text: L10n.tr("phone.find.camera.denied"))
                        Button(L10n.tr("phone.common.open_settings")) {
                            if let url = URL(string: UIApplication.openSettingsURLString) {
                                self.openURL(url)
                            }
                        }
                        .buttonStyle(.phoneSecondary)
                        .fixedSize()
                    }
                }
            }
            .aspectRatio(1, contentMode: .fit)
            .frame(maxWidth: 290)
            .clipShape(RoundedRectangle(cornerRadius: BobeRadius.sheet, style: .continuous))
            Button(L10n.tr("phone.find.type_instead")) {
                self.model.clearPairingError()
                self.unknownCode = false
                self.typing = true
                self.focus = .address
            }
            .buttonStyle(.phoneQuiet)
        }
    }

    private var manualForm: some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 6) {
                PhoneSectionLabel(text: L10n.tr("phone.find.address.label"))
                TextField(L10n.tr("phone.find.address.placeholder"), text: self.$address)
                    .keyboardType(.URL)
                    .textContentType(.URL)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.next)
                    .focused(self.$focus, equals: .address)
                    .onSubmit { self.focus = .code }
                    .padding(.horizontal, 16)
                    .frame(minHeight: 52)
                    .bobeWell(cornerRadius: BobeRadius.field)
                    .accessibilityIdentifier("phone.find.address")
            }
            VStack(alignment: .leading, spacing: 6) {
                PhoneSectionLabel(text: L10n.tr("phone.find.code.label"))
                CodeBoxes(code: self.$code, focus: self.$focus)
            }
            Button(L10n.tr("phone.find.pair")) { self.submit() }
                .buttonStyle(.phonePrimary)
                .disabled(self.invite == nil)
                .accessibilityIdentifier("phone.find.pair")
            if CodeScanner.isAvailable {
                Button(L10n.tr("phone.find.scan_instead")) {
                    self.model.clearPairingError()
                    self.focus = nil
                    self.typing = false
                }
                .buttonStyle(.phoneQuiet)
                .frame(maxWidth: .infinity)
            }
        }
    }

    private func cameraMessage(symbol: String, text: String) -> some View {
        VStack(spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 34, weight: .medium))
                .foregroundStyle(self.theme.colors.textMuted)
            Text(text)
                .phoneText(.callout)
                .foregroundStyle(self.theme.colors.textMuted)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(24)
    }

    private func notice(_ text: String) -> some View {
        Label(text, systemImage: "exclamationmark.circle.fill")
            .phoneText(.callout)
            .foregroundStyle(self.theme.colors.warning)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(14)
            .background(
                self.theme.colors.warning.opacity(0.1),
                in: RoundedRectangle(cornerRadius: BobeRadius.control, style: .continuous)
            )
            .transition(.bobeRise)
    }

    private func scanned(_ value: String) {
        self.model.clearPairingError()
        guard let url = URL(string: value), let invite = PairingInvite(link: url) else {
            self.unknownCode = true
            return
        }
        self.unknownCode = false
        Task { await self.model.pair(invite) }
    }

    private func submit() {
        guard let invite = self.invite else { return }
        self.focus = nil
        self.model.clearPairingError()
        Task { await self.model.pair(invite) }
    }
}

private struct CodeBoxes: View {
    @Binding var code: String
    var focus: FocusState<FindStep.Field?>.Binding
    @Environment(\.theme) private var theme

    var body: some View {
        ZStack {
            TextField("", text: self.$code)
                .keyboardType(.numberPad)
                .textContentType(.oneTimeCode)
                .focused(self.focus, equals: .code)
                .opacity(0.02)
                .frame(width: 1, height: 1)
                .accessibilityLabel(L10n.tr("phone.find.code.label"))
                .accessibilityIdentifier("phone.find.code")
            HStack(spacing: 8) {
                ForEach(0 ..< 6, id: \.self) { index in
                    let digits = Array(self.code)
                    let current = self.focus.wrappedValue == .code && index == min(digits.count, 5)
                    Text(index < digits.count ? String(digits[index]) : "")
                        .phoneText(.digits)
                        .foregroundStyle(self.theme.colors.text)
                        .frame(maxWidth: .infinity, minHeight: 56)
                        .bobeWell(cornerRadius: BobeRadius.control)
                        .overlay {
                            RoundedRectangle(cornerRadius: BobeRadius.control, style: .continuous)
                                .strokeBorder(self.theme.colors.primary, lineWidth: current ? 2 : 0)
                        }
                }
            }
            .contentShape(Rectangle())
            .onTapGesture { self.focus.wrappedValue = .code }
            .accessibilityHidden(true)
        }
        .onChange(of: self.code) { _, value in
            let digits = String(value.unicodeScalars.filter { ("0" ... "9").contains($0) }.prefix(6))
            if digits != value {
                self.code = digits
            }
        }
    }
}

private struct PairingStatusCard: View {
    let text: String
    let done: Bool
    @Environment(\.theme) private var theme

    var body: some View {
        PhoneCard(padding: 22) {
            HStack(spacing: 14) {
                if self.done {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: 26))
                        .foregroundStyle(self.theme.colors.success)
                        .transition(.bobePop())
                } else {
                    ProgressView()
                        .controlSize(.regular)
                }
                if self.done {
                    Text(self.text)
                        .phoneText(.heading)
                        .foregroundStyle(self.theme.colors.text)
                } else {
                    BobeShimmerText(text: self.text, font: PhoneText.heading.font)
                }
            }
            .frame(maxWidth: .infinity)
        }
        .padding(.top, 10)
    }
}

private struct WhereIsTheCodeSheet: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.theme) private var theme

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(L10n.tr("phone.where.title"))
                .phoneText(.title)
                .foregroundStyle(self.theme.colors.text)
                .padding(.top, 28)
            PhoneCard {
                VStack(alignment: .leading, spacing: 14) {
                    OnboardingPoint(symbol: "laptopcomputer", text: L10n.tr("phone.where.mac"))
                    HStack(alignment: .center, spacing: 12) {
                        BobeIconTile(systemName: "terminal.fill", tint: self.theme.colors.primary, size: 30)
                        Text(L10n.tr("phone.where.linux"))
                            .phoneText(.callout)
                            .foregroundStyle(self.theme.colors.text)
                        Text(verbatim: "bobe device pair --kind phone")
                            .font(.system(.footnote, design: .monospaced))
                            .lineLimit(1)
                            .minimumScaleFactor(0.6)
                            .foregroundStyle(self.theme.colors.text)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 5)
                            .bobeWell(cornerRadius: BobeRadius.chip)
                            .textSelection(.enabled)
                    }
                }
            }
            Spacer(minLength: 0)
            Button(L10n.tr("phone.common.done")) { self.dismiss() }
                .buttonStyle(.phoneSecondary)
        }
        .padding(.horizontal, 24)
        .padding(.bottom, 12)
        .background(PhoneBackdrop())
        .phoneThemed()
    }
}
