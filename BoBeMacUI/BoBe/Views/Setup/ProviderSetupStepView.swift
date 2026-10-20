import SwiftUI

struct ProviderSetupStepView: View {
    @Binding var fields: ProviderConfigurationFields
    @Binding var apiKey: String
    let onContinue: () -> Void
    let onCriticalOperationChange: (Bool) -> Void

    @Environment(\.theme) private var theme
    @State private var isPreparing = false
    @State private var connectionReady = false
    @State private var resumed = false

    var body: some View {
        VStack(alignment: .trailing, spacing: 16) {
            SetupStepHeader(title: L10n.tr("setup.provider.title"), message: L10n.tr("setup.provider.body"))
                .padding(.top, 8)
            ScrollView {
                if self.resumed {
                    ProviderConfigurationForm(
                        fields: self.$fields,
                        apiKey: self.$apiKey,
                        onCriticalOperationChange: {
                            self.isPreparing = $0
                            self.onCriticalOperationChange($0)
                        },
                        onConnectionStateChange: { self.connectionReady = $0 }
                    )
                    .padding(.horizontal, 2)
                    .padding(.bottom, 8)
                    .transition(.bobeRise)
                }
            }
            .scrollIndicators(.never)
            .task { await self.resumeSavedAccess() }
            Button(L10n.tr("setup.welcome.continue"), action: self.onContinue)
                .bobeButton(.primary, size: .regular)
                .keyboardShortcut(.defaultAction)
                .disabled(self.isPreparing || !self.connectionReady || !self.fields.canActivate(apiKey: self.apiKey))
        }
    }

    private func resumeSavedAccess() async {
        defer {
            withAnimation(BobeMotion.standard) { self.resumed = true }
        }
        guard !self.fields.isConfigured else { return }
        let selection = EndpointStore.shared.selection
        guard let response = try? await DaemonClient(endpoint: selection.endpoint).listProviders(),
              EndpointStore.shared.selection == selection,
              let saved = response.profiles.first(where: { $0.credentialPresent && $0.localRuntime == nil })
        else { return }
        self.fields.load(response, profileID: saved.id)
    }
}
