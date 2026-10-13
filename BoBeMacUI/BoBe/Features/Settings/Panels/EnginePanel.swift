import SwiftUI

struct EnginePanel: View {
    @Environment(SettingsStore.self) private var store
    @Environment(\.theme) private var theme
    @State private var section: EngineSection = .models

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SettingsSectionDock(selection: self.$section)
                .padding(.horizontal, 24)
                .padding(.top, 20)
                .padding(.bottom, 16)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    if let error = self.store.error {
                        SettingsErrorBanner(message: error)
                    }
                    switch self.section {
                    case .models: ProviderProfilesSection()
                    case .activity: AgentWorkSection(showsTools: false, showsWork: true)
                    case .capabilities: AgentWorkSection(showsTools: true, showsWork: false)
                    }
                }
                .padding(.horizontal, 24)
                .padding(.bottom, 24)
            }
        }
    }
}
