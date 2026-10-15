import SwiftUI

struct OnboardingView: View {
    @Bindable var model: PhoneModel
    @Environment(\.theme) private var theme
    @State private var awake = false

    private var mood: BobeMood {
        switch self.model.pairing {
        case .working: return .thinking
        case .paired: return .happy
        case .failed: return .worried
        case .idle: break
        }
        switch self.model.step {
        case .welcome: return self.awake ? .idle : .waking
        case .find: return .watching
        case .hear: return .listening
        case .ready: return .happy
        }
    }

    private var faceSize: CGFloat {
        switch self.model.step {
        case .welcome: 172
        case .find: 84
        case .hear, .ready: 132
        }
    }

    var body: some View {
        ZStack {
            PhoneBackdrop()
            GeometryReader { proxy in
                ScrollView {
                    self.content
                        .frame(minHeight: proxy.size.height)
                }
                .scrollBounceBehavior(.basedOnSize)
                .scrollDismissesKeyboard(.interactively)
            }
        }
        .task {
            try? await Task.sleep(for: .seconds(1.1))
            self.awake = true
        }
    }

    private var content: some View {
        VStack(spacing: 0) {
            OnboardingProgress(step: self.model.step)
                .padding(.top, 8)
            if self.model.step == .welcome {
                Spacer(minLength: 0)
                    .frame(maxHeight: 90)
            }
            BobeCharacter(
                mood: self.mood,
                size: self.faceSize,
                palette: self.theme.avatar
            )
            .padding(.top, self.model.step == .find ? 6 : 28)
            .animation(BobeMotion.soft, value: self.faceSize)
            Group {
                switch self.model.step {
                case .welcome: WelcomeStep(model: self.model)
                case .find: FindStep(model: self.model)
                case .hear: HearStep(model: self.model)
                case .ready: ReadyStep(model: self.model)
                }
            }
            .id(self.model.step)
            .transition(.bobeRise)
        }
        .padding(.horizontal, 24)
        .padding(.bottom, 8)
    }
}

private struct OnboardingProgress: View {
    let step: PhoneModel.Step
    @Environment(\.theme) private var theme

    var body: some View {
        HStack(spacing: 6) {
            ForEach(PhoneModel.Step.allCases, id: \.self) { item in
                Capsule(style: .continuous)
                    .fill(item.rawValue <= self.step.rawValue ? self.theme.colors.primary : self.theme.colors.border)
                    .frame(width: item == self.step ? 22 : 7, height: 7)
            }
        }
        .animation(BobeMotion.standard, value: self.step)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(
            L10n.tr("phone.onboarding.progress_format", self.step.rawValue + 1, PhoneModel.Step.allCases.count)
        )
    }
}

struct OnboardingCopy: View {
    let title: String
    let message: String
    @Environment(\.theme) private var theme

    var body: some View {
        VStack(spacing: 10) {
            Text(self.title)
                .phoneText(.title)
                .foregroundStyle(self.theme.colors.text)
            Text(self.message)
                .phoneText(.body)
                .foregroundStyle(self.theme.colors.textMuted)
                .lineSpacing(2)
        }
        .multilineTextAlignment(.center)
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: .infinity)
    }
}

struct OnboardingPoint: View {
    let symbol: String
    let text: String
    @Environment(\.theme) private var theme

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            BobeIconTile(systemName: self.symbol, tint: self.theme.colors.primary, size: 30)
            Text(self.text)
                .phoneText(.callout)
                .foregroundStyle(self.theme.colors.text)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
