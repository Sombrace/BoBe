import SwiftUI

struct QuestionCard: View {
    let model: PhoneModel
    let question: OwnerInputWire
    @State private var reply = ""
    @State private var sending = false
    @State private var failed = false
    @Environment(\.theme) private var theme

    private var trimmedReply: String {
        self.reply.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label(L10n.tr("phone.question.label"), systemImage: "questionmark.bubble.fill")
                .phoneText(.label)
                .foregroundStyle(self.theme.colors.primary)
            Text(self.question.question)
                .phoneText(.heading)
                .foregroundStyle(self.theme.colors.text)
                .fixedSize(horizontal: false, vertical: true)
            if !self.question.prompt.isEmpty, self.question.prompt != self.question.question {
                Text(self.question.prompt)
                    .phoneText(.callout)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if self.question.choices.isEmpty {
                HStack(alignment: .bottom, spacing: 8) {
                    TextField(L10n.tr("phone.question.placeholder"), text: self.$reply, axis: .vertical)
                        .lineLimit(1 ... 4)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 11)
                        .bobeWell(cornerRadius: BobeRadius.field)
                    Button {
                        self.submit(self.trimmedReply)
                    } label: {
                        Image(systemName: "arrow.up.circle.fill")
                            .font(.system(size: 34))
                            .foregroundStyle(self.theme.colors.primary)
                    }
                    .disabled(self.trimmedReply.isEmpty || self.sending)
                    .accessibilityLabel(L10n.tr("phone.question.send"))
                }
            } else {
                VStack(spacing: 8) {
                    ForEach(self.question.choices) { choice in
                        Button(choice.label) { self.submit(choice.value) }
                            .buttonStyle(.phoneSecondary)
                            .disabled(self.sending)
                    }
                }
            }
            if self.failed {
                Text(L10n.tr("phone.question.failed"))
                    .phoneText(.footnote)
                    .foregroundStyle(self.theme.colors.warning)
            }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .bobeRaised(cornerRadius: BobeRadius.card, elevation: .lifted)
        .overlay {
            RoundedRectangle(cornerRadius: BobeRadius.card, style: .continuous)
                .strokeBorder(self.theme.colors.primary.opacity(0.35), lineWidth: 1.5)
        }
        .onAppear { self.model.presented(self.question) }
        .transition(.bobeRise)
    }

    private func submit(_ response: String) {
        guard !response.isEmpty, !self.sending else { return }
        self.sending = true
        self.failed = false
        Task {
            let answered = await self.model.answer(self.question, with: response)
            self.sending = false
            self.failed = !answered
        }
    }
}
