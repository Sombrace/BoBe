import AppKit
import SwiftUI

struct OwnerQuestionsView: View {
    let selection: EndpointSelection
    @Environment(\.theme) private var theme
    @State private var questions: [OwnerInputWire] = []
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let question = self.questions.first(where: { $0.status == .pending }) {
                OwnerQuestionRow(question: question, selection: self.selection, onAnswered: self.refresh)
                    .id(question.id)
                    .transition(.bobeRise)
            }
            if let error {
                Text(error).bobeTextStyle(.helper).foregroundStyle(self.theme.colors.error)
            }
        }
        .animation(BobeMotion.standard, value: self.questions.first(where: { $0.status == .pending })?.id)
        .task(id: self.selection) {
            self.questions = []
            self.error = nil
            while !Task.isCancelled {
                await self.refresh()
                do { try await Task.sleep(for: .seconds(5)) } catch { return }
            }
        }
    }

    private func refresh() async {
        guard EndpointStore.shared.selection == self.selection else { return }
        do {
            let questions = try await DaemonClient(endpoint: self.selection.endpoint).ownerQuestions()
            guard EndpointStore.shared.selection == self.selection else { return }
            self.questions = questions
            self.error = nil
        } catch {
            guard !Task.isCancelled, EndpointStore.shared.selection == self.selection else { return }
            self.error = error.localizedDescription
        }
    }
}

private struct OwnerQuestionRow: View {
    let question: OwnerInputWire
    let selection: EndpointSelection
    let onAnswered: @MainActor () async -> Void
    @Environment(\.theme) private var theme
    @State private var answer = ""
    @State private var submittedAnswer: String?
    @State private var responseId = UUID()
    @State private var working = false
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                BobeIconTile(systemName: "hand.raised.fill", tint: self.theme.colors.primary, size: 24)
                Text(L10n.tr("overlay.owner_input.title"))
                    .font(.system(size: 12.5, weight: .semibold))
                    .foregroundStyle(self.theme.colors.primary)
                Spacer(minLength: 0)
                if self.working { BobeSpinner(size: 12) }
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 6) {
                    if !self.question.question.contains(self.question.action) {
                        Text(self.question.action)
                            .bobeTextStyle(.helper)
                            .foregroundStyle(self.theme.colors.textMuted)
                    }
                    Text(self.question.question)
                        .bobeTextStyle(.chatBody)
                        .lineSpacing(2)
                        .foregroundStyle(self.theme.colors.text)
                        .textSelection(.enabled)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .scrollBounceBehavior(.basedOnSize)
            .frame(maxHeight: 140)
            .fixedSize(horizontal: false, vertical: true)
            if self.question.choices.isEmpty {
                HStack(alignment: .bottom, spacing: 6) {
                    TextField(L10n.tr("overlay.owner_input.answer"), text: self.$answer, axis: .vertical)
                        .textFieldStyle(.plain)
                        .bobeTextStyle(.inputField)
                        .lineLimit(1 ... 4)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .bobeWell(cornerRadius: BobeRadius.field)
                        .onSubmit { Task { await self.submit(self.answer) } }
                    Button {
                        Task { await self.submit(self.answer) }
                    } label: {
                        Image(systemName: "arrow.up")
                            .font(.system(size: 12, weight: .bold))
                            .foregroundStyle(self.theme.onAccent)
                            .frame(width: 32, height: 32)
                            .background(Circle().fill(self.theme.colors.primary))
                    }
                    .buttonStyle(BobePressStyle(scale: 0.9))
                    .accessibilityLabel(L10n.tr("overlay.owner_input.send"))
                    .disabled(self.answer.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }
            } else {
                FlowLayout(spacing: 6) { self.choices }
            }
            if let error {
                Text(error).bobeTextStyle(.helper).foregroundStyle(self.theme.colors.error)
            }
        }
        .padding(12)
        .bobeRaised(cornerRadius: BobeRadius.card, elevation: .lifted, fill: self.theme.accentSoft)
        .disabled(self.working)
        .task(id: self.question.id) { await self.presented() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            Task { await self.presented() }
        }
    }

    @ViewBuilder
    private var choices: some View {
        ForEach(self.question.choices) { choice in
            Button(choice.label) { Task { await self.submit(choice.value) } }
                .bobeButton(.secondary, size: .small)
                .accessibilityHint(self.question.action)
        }
    }

    private func presented() async {
        guard NSApp.isActive, EndpointStore.shared.selection == self.selection else { return }
        do { try await DaemonClient(endpoint: self.selection.endpoint).markOwnerQuestionPresented(self.question.id) }
        catch {
            guard EndpointStore.shared.selection == self.selection else { return }
            self.error = error.localizedDescription
        }
    }

    private func submit(_ response: String) async {
        guard !self.working, !response.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              EndpointStore.shared.selection == self.selection else { return }
        if self.submittedAnswer != response {
            self.responseId = UUID()
            self.submittedAnswer = response
        }
        self.working = true
        defer { self.working = false }
        do {
            _ = try await DaemonClient(endpoint: self.selection.endpoint)
                .answerOwnerQuestion(self.question.id, response: response, responseId: self.responseId)
            guard EndpointStore.shared.selection == self.selection else { return }
            self.error = nil
            await self.onAnswered()
        } catch {
            guard EndpointStore.shared.selection == self.selection else { return }
            self.error = error.localizedDescription
        }
    }
}
