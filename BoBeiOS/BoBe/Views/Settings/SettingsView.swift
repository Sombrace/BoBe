import SwiftUI
import UIKit

struct SettingsView: View {
    @Bindable var model: PhoneModel
    @State private var confirmsUnpair = false
    @State private var voice = PhonePermissions.voice()
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    @Environment(\.theme) private var theme

    private var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? ""
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    HStack(spacing: 14) {
                        BobeCharacter(mood: self.model.mood, size: 44, palette: self.theme.avatar)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(verbatim: "BoBe")
                                .phoneText(.heading)
                                .foregroundStyle(self.theme.colors.text)
                            Text(self.model.status)
                                .phoneText(.footnote)
                                .foregroundStyle(self.theme.colors.textMuted)
                        }
                    }
                    .padding(.vertical, 4)
                    LabeledContent(L10n.tr("phone.settings.address")) {
                        Text(self.model.brainAddress)
                            .font(.system(.footnote, design: .monospaced))
                            .foregroundStyle(self.theme.colors.textMuted)
                            .textSelection(.enabled)
                    }
                    if [.offline, .incompatible, .settingUp].contains(self.model.link) {
                        Button(L10n.tr("phone.connection.retry")) { self.model.reconnect() }
                    }
                }
                .listRowBackground(self.theme.colors.surface)

                if self.model.forgetPending {
                    Section {
                        Text(L10n.tr("phone.settings.forget_pending"))
                            .phoneText(.footnote)
                            .foregroundStyle(self.theme.colors.textMuted)
                        Button(L10n.tr("phone.connection.retry")) { self.model.retryForgetting() }
                    }
                    .listRowBackground(self.theme.colors.surface)
                }

                Section {
                    LabeledContent(L10n.tr("phone.settings.voice"), value: self.describe(self.voice))
                    Button(L10n.tr("phone.common.open_settings")) {
                        if let url = URL(string: UIApplication.openSettingsURLString) {
                            self.openURL(url)
                        }
                    }
                }
                .listRowBackground(self.theme.colors.surface)

                Section {
                    Button(L10n.tr("phone.settings.unpair"), role: .destructive) { self.confirmsUnpair = true }
                        .accessibilityIdentifier("phone.settings.unpair")
                        .confirmationDialog(
                            L10n.tr("phone.settings.unpair.title"),
                            isPresented: self.$confirmsUnpair,
                            titleVisibility: .visible
                        ) {
                            Button(L10n.tr("phone.settings.unpair.confirm"), role: .destructive) { self.model.unpair() }
                        } message: {
                            Text(L10n.tr("phone.settings.unpair.message"))
                        }
                }
                .listRowBackground(self.theme.colors.surface)

                Section {
                    LabeledContent(L10n.tr("phone.settings.version"), value: self.version)
                }
                .listRowBackground(self.theme.colors.surface)
            }
            .scrollContentBackground(.hidden)
            .background(PhoneBackdrop())
            .navigationTitle(L10n.tr("phone.settings.title"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button(L10n.tr("phone.common.done")) { self.dismiss() }
                }
            }
            .task {
                self.voice = PhonePermissions.voice()
            }
        }
        .phoneThemed()
    }

    private func describe(_ permission: PhonePermission) -> String {
        switch permission {
        case .granted: L10n.tr("phone.settings.permission.on")
        case .denied: L10n.tr("phone.settings.permission.off")
        case .unknown: L10n.tr("phone.settings.permission.unset")
        }
    }
}
