import SwiftUI

struct WelcomeValueStepView: View {
    let onContinue: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Spacer(minLength: 0)

            SetupStepHeader(
                title: L10n.tr("setup.value.title"),
                message: L10n.tr("setup.value.body")
            )
            .setupStagger(0)

            Spacer(minLength: 0)

            Button(action: self.onContinue) {
                Label(L10n.tr("setup.value.continue"), systemImage: "arrow.right")
                    .labelStyle(TrailingIconLabelStyle())
            }
            .bobeButton(.primary)
            .keyboardShortcut(.defaultAction)
            .accessibilityIdentifier("setup.value.continue")
            .frame(maxWidth: .infinity, alignment: .trailing)
            .setupStagger(1)
        }
    }
}

struct TrailingIconLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 6) {
            configuration.title
            configuration.icon
        }
    }
}
