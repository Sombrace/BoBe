import SwiftUI

struct BobeSegmented<Option: Hashable>: View {
    @Binding var selection: Option
    let options: [Option]
    let title: (Option) -> String
    var height: CGFloat = 30

    @Environment(\.theme) private var theme
    @Environment(\.isEnabled) private var isEnabled
    @Namespace private var thumb

    var body: some View {
        HStack(spacing: 2) {
            ForEach(self.options, id: \.self) { option in
                Button {
                    withAnimation(BobeMotion.snappy) {
                        self.selection = option
                    }
                } label: {
                    Text(self.title(option))
                        .font(.system(size: 12.5, weight: .medium))
                        .lineLimit(1)
                        .foregroundStyle(self.selection == option ? self.theme.colors.text : self.theme.colors.textMuted)
                        .padding(.horizontal, 12)
                        .frame(maxWidth: .infinity)
                        .frame(height: self.height - 6)
                        .background {
                            if self.selection == option {
                                Capsule(style: .circular)
                                    .fill(self.theme.colors.surface)
                                    .shadow(color: self.theme.shadowTint.opacity(0.14), radius: 2.5, y: 1.5)
                                    .matchedGeometryEffect(id: "selection", in: self.thumb)
                            }
                        }
                        .contentShape(Capsule(style: .circular))
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(self.selection == option ? [.isSelected] : [])
            }
        }
        .padding(3)
        .bobeWell(in: .bobePill)
        .opacity(self.isEnabled ? 1 : 0.5)
        .accessibilityElement(children: .contain)
    }
}
