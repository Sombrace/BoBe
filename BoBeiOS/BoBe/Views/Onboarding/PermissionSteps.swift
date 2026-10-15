import SwiftUI

struct HearStep: View {
    @Bindable var model: PhoneModel
    @State private var asking = false

    var body: some View {
        OnboardingAsk(
            title: L10n.tr("phone.hear.title"),
            message: L10n.tr("phone.hear.body"),
            allowTitle: L10n.tr("phone.hear.allow"),
            skipTitle: L10n.tr("phone.common.maybe_later"),
            asking: self.asking,
            onAllow: {
                self.asking = true
                Task {
                    await self.model.allowVoice()
                    self.asking = false
                }
            },
            onSkip: self.model.skipVoice
        )
    }
}

struct ReadyStep: View {
    @Bindable var model: PhoneModel
    @Environment(\.theme) private var theme

    var body: some View {
        VStack(spacing: 0) {
            Text(L10n.tr("phone.ready.title"))
                .phoneText(.title)
                .foregroundStyle(self.theme.colors.text)
                .padding(.top, 22)
            Spacer(minLength: 24)
            Button(L10n.tr("phone.ready.start")) { self.model.finishOnboarding() }
                .buttonStyle(.phonePrimary)
                .accessibilityIdentifier("phone.ready.start")
                .padding(.bottom, 50)
        }
    }
}

private struct OnboardingAsk: View {
    let title: String
    let message: String
    let allowTitle: String
    let skipTitle: String
    let asking: Bool
    let onAllow: () -> Void
    let onSkip: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            OnboardingCopy(title: self.title, message: self.message)
                .padding(.top, 22)
            Spacer(minLength: 20)
            VStack(spacing: 6) {
                Button(self.allowTitle, action: self.onAllow)
                    .buttonStyle(.phonePrimary)
                    .disabled(self.asking)
                Button(self.skipTitle, action: self.onSkip)
                    .buttonStyle(.phoneQuiet)
                    .disabled(self.asking)
            }
        }
    }
}
