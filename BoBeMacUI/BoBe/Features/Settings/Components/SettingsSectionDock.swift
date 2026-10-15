import SwiftUI

enum EngineSection: String, CaseIterable, Identifiable {
    case models, activity, capabilities
    var id: String { self.rawValue }
    var title: String {
        switch self {
        case .models: L10n.tr("settings.providers.section.models")
        case .activity: L10n.tr("settings.providers.section.activity")
        case .capabilities: L10n.tr("settings.providers.section.capabilities")
        }
    }
    var symbol: String {
        switch self {
        case .models: "sparkles"
        case .activity: "waveform.path"
        case .capabilities: "switch.2"
        }
    }
}

struct SettingsSectionDock: View {
    @Binding var selection: EngineSection
    @Environment(\.theme) private var theme
    @Namespace private var thumb

    var body: some View {
        HStack(spacing: 2) {
            ForEach(EngineSection.allCases) { section in
                Button {
                    withAnimation(BobeMotion.snappy) {
                        self.selection = section
                    }
                } label: {
                    Label(section.title, systemImage: section.symbol)
                        .font(.system(size: 13, weight: .medium))
                        .foregroundStyle(self.selection == section ? self.theme.colors.text : self.theme.colors.textMuted)
                        .padding(.horizontal, 14)
                        .frame(height: 30)
                        .background {
                            if self.selection == section {
                                Capsule(style: .circular)
                                    .fill(self.theme.colors.surface)
                                    .shadow(color: self.theme.shadowTint.opacity(0.14), radius: 2.5, y: 1.5)
                                    .matchedGeometryEffect(id: "selection", in: self.thumb)
                            }
                        }
                        .contentShape(Capsule(style: .circular))
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(self.selection == section ? [.isSelected] : [])
            }
        }
        .padding(3)
        .bobeWell(in: .bobePill)
        .accessibilityElement(children: .contain)
    }
}
