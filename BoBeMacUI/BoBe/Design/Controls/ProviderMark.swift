import SwiftUI

struct ProviderMark: View {
    let kind: ProviderKindWire
    var size: CGFloat = 34
    @Environment(\.theme) private var theme

    var body: some View {
        Group {
            if let asset = self.kind.logoAssetName {
                Image(asset, bundle: .appResources)
                    .resizable()
                    .renderingMode(self.kind.usesMonochromeLogo ? .template : .original)
                    .scaledToFit()
                    .foregroundStyle(self.theme.isDark ? Color.white : Color.black)
                    .frame(width: self.markSize, height: self.markSize)
            } else {
                Image(systemName: "server.rack")
                    .font(.system(size: self.size * 0.44, weight: .medium))
                    .foregroundStyle(self.theme.colors.textMuted)
            }
        }
        .frame(width: self.size, height: self.size)
        .background(
            RoundedRectangle(cornerRadius: self.size * 0.3, style: .continuous)
                .fill(self.theme.colors.surface)
        )
        .overlay(
            RoundedRectangle(cornerRadius: self.size * 0.3, style: .continuous)
                .stroke(self.theme.colors.border.opacity(0.6), lineWidth: 0.5)
        )
        .accessibilityHidden(true)
    }

    private var markSize: CGFloat {
        self.size <= 20 ? self.size : self.size * 0.68
    }
}

extension ProviderKindWire {
    var logoAssetName: String? {
        switch self {
        case .openAI: "ProviderOpenAI"
        case .anthropic: "ProviderAnthropic"
        case .deepSeek: "ProviderDeepSeek"
        case .openRouter: "ProviderOpenRouter"
        case .copilot: "ProviderCopilot"
        case .railRelay: "ProviderRailRelay"
        case .openAICompatible: nil
        }
    }

    var usesMonochromeLogo: Bool {
        self == .openAI || self == .copilot
    }
}
