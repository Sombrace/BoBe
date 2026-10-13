import SwiftUI

struct WelcomeStep: View {
    @Bindable var model: PhoneModel
    @State private var showsHelp = false

    var body: some View {
        VStack(spacing: 0) {
            OnboardingCopy(
                title: L10n.tr("phone.welcome.title"),
                message: L10n.tr("phone.welcome.body")
            )
            .padding(.top, 26)
            Spacer(minLength: 24)
            VStack(spacing: 6) {
                Button(L10n.tr("phone.welcome.find")) { self.model.begin() }
                    .buttonStyle(.phonePrimary)
                    .accessibilityIdentifier("phone.welcome.find")
                Button(L10n.tr("phone.welcome.no_bobe")) { self.showsHelp = true }
                    .buttonStyle(.phoneQuiet)
            }
        }
        .sheet(isPresented: self.$showsHelp) {
            GetBoBeSheet()
                .fittedSheet()
        }
    }
}

private struct GetBoBeSheet: View {
    @Environment(\.dismiss) private var dismiss
    @Environment(\.theme) private var theme

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text(L10n.tr("phone.get.title"))
                .phoneText(.title)
                .foregroundStyle(self.theme.colors.text)
                .padding(.top, 28)
            Text(L10n.tr("phone.get.body"))
                .phoneText(.body)
                .foregroundStyle(self.theme.colors.textMuted)
                .fixedSize(horizontal: false, vertical: true)
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
