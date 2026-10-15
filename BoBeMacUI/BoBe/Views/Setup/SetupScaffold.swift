import SwiftUI

struct SetupStepHeader: View {
    let title: String
    var message: String?
    var alignment: HorizontalAlignment = .leading
    @Environment(\.theme) private var theme

    var body: some View {
        VStack(alignment: self.alignment, spacing: 8) {
            Text(self.title)
                .bobeTextStyle(.setupTitle)
                .foregroundStyle(self.theme.colors.text)
                .fixedSize(horizontal: false, vertical: true)
            if let message {
                Text(message)
                    .bobeTextStyle(.setupBody)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .lineSpacing(3)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .multilineTextAlignment(self.alignment == .center ? .center : .leading)
        .frame(maxWidth: .infinity, alignment: self.alignment == .center ? .center : .leading)
    }
}

struct SetupChoiceCard<Detail: View>: View {
    let icon: String
    let tint: Color
    let title: String
    var subtitle: String?
    let isSelected: Bool
    let action: () -> Void
    @ViewBuilder let detail: Detail

    @Environment(\.theme) private var theme
    @State private var isHovered = false

    private var shape: RoundedRectangle {
        RoundedRectangle(cornerRadius: BobeRadius.card, style: .continuous)
    }

    var body: some View {
        Button(action: self.action) {
            HStack(alignment: .top, spacing: 13) {
                BobeIconTile(systemName: self.icon, tint: self.isSelected ? self.tint : self.theme.colors.textMuted, size: 36)
                VStack(alignment: .leading, spacing: 4) {
                    Text(self.title)
                        .bobeTextStyle(.heading)
                        .foregroundStyle(self.theme.colors.text)
                    if let subtitle {
                        Text(subtitle)
                            .bobeTextStyle(.helper)
                            .foregroundStyle(self.theme.colors.textMuted)
                            .multilineTextAlignment(.leading)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    self.detail
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                ZStack {
                    Circle()
                        .strokeBorder(self.theme.colors.border, lineWidth: 1.5)
                        .opacity(self.isSelected ? 0 : 1)
                    if self.isSelected {
                        Image(systemName: "checkmark.circle.fill")
                            .font(.system(size: 22))
                            .foregroundStyle(self.theme.colors.primary)
                            .transition(.bobePop())
                    }
                }
                .frame(width: 22, height: 22)
                .padding(.top, 7)
            }
            .padding(14)
            .bobeRaised(in: self.shape, elevation: self.isHovered || self.isSelected ? .lifted : .resting)
            .overlay {
                self.shape.strokeBorder(self.theme.colors.primary.opacity(self.isSelected ? 0.85 : 0), lineWidth: 1.5)
            }
            .offset(y: self.isHovered && !self.isSelected ? -1.5 : 0)
            .contentShape(self.shape)
        }
        .buttonStyle(BobePressStyle(scale: 0.985))
        .accessibilityAddTraits(self.isSelected ? .isSelected : [])
        .animation(BobeMotion.standard, value: self.isSelected)
        .animation(BobeMotion.standard, value: self.isHovered)
        .onHover { self.isHovered = $0 }
    }
}

extension SetupChoiceCard where Detail == EmptyView {
    init(
        icon: String,
        tint: Color,
        title: String,
        subtitle: String? = nil,
        isSelected: Bool,
        action: @escaping () -> Void
    ) {
        self.init(
            icon: icon, tint: tint, title: title, subtitle: subtitle, isSelected: isSelected, action: action
        ) { EmptyView() }
    }
}

struct SetupStagger: ViewModifier {
    let index: Int
    @State private var shown = false

    func body(content: Content) -> some View {
        content
            .opacity(self.shown ? 1 : 0)
            .offset(y: self.shown ? 0 : 10)
            .onAppear {
                withAnimation(BobeMotion.stagger(self.index, step: 0.07)) {
                    self.shown = true
                }
            }
    }
}

extension View {
    func setupStagger(_ index: Int) -> some View {
        modifier(SetupStagger(index: index))
    }
}
