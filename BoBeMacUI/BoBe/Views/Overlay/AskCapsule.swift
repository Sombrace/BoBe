import SwiftUI

struct AskCapsule: View {
    @Binding var text: String
    let onSend: (String) -> Bool
    let onClose: () -> Void
    let selection: EndpointSelection
    var feedbackMessage: String?
    var isWorking = false
    var canStopVoice = false
    var onStopVoice: () -> Void = {}

    @FocusState private var isFocused: Bool
    @Environment(\.theme) private var theme
    @Environment(\.overlayPlacement) private var placement
    @State private var runs = AgentRunMonitor()

    private var shape: RoundedRectangle {
        RoundedRectangle(cornerRadius: BobeRadius.capsule, style: .circular)
    }

    private var hasText: Bool {
        self.text.contains(where: { !$0.isWhitespace })
    }

    private var isSteering: Bool {
        self.runs.activeRun != nil && self.hasText
    }

    private var showsStop: Bool {
        !self.hasText && (self.runs.activeRun != nil || self.canStopVoice)
    }

    var body: some View {
        AnchoredStack(
            hugsTrailing: self.placement.hugsTrailing,
            reversed: self.placement.growth == .down,
            spacing: 6
        ) {
            if self.isSteering {
                SteerModeChooser(appliesLater: self.$runs.appliesAfterCurrentTask)
                    .transition(.bobePop(anchor: self.placement.corner))
            }
            if let note = self.runs.error ?? self.feedbackMessage {
                self.note(note, isError: self.runs.error != nil)
                    .transition(.bobeRise)
            }
            self.field
        }
        .animation(BobeMotion.standard, value: self.isSteering)
        .animation(BobeMotion.standard, value: self.feedbackMessage)
        .task(id: self.selection) {
            await self.runs.watch(self.selection)
        }
        .task(id: OverlayWindowManager.shared.stage.focusRequest) {
            self.isFocused = true
            try? await Task.sleep(for: .milliseconds(60))
            self.isFocused = true
        }
    }

    private var field: some View {
        HStack(alignment: .bottom, spacing: 6) {
            ZStack(alignment: .leading) {
                if self.text.isEmpty {
                    Text(L10n.tr("overlay.input.placeholder"))
                        .bobeTextStyle(.inputField)
                        .foregroundStyle(self.theme.colors.textMuted.opacity(0.8))
                        .allowsHitTesting(false)
                        .accessibilityHidden(true)
                }
                TextField("", text: self.$text, axis: .vertical)
                    .accessibilityLabel(L10n.tr("overlay.input.composer.accessibility"))
                    .accessibilityIdentifier("overlay.composer.field")
                    .textFieldStyle(.plain)
                    .bobeTextStyle(.inputField)
                    .lineSpacing(2)
                    .foregroundStyle(self.theme.colors.text)
                    .tint(self.theme.colors.primary)
                    .lineLimit(1 ... 6)
                    .focused(self.$isFocused)
                    .onSubmit { self.submit() }
                    .onKeyPress(.escape) {
                        self.onClose()
                        return .handled
                    }
            }
            .padding(.leading, 12)
            .padding(.vertical, 8)
            .frame(maxWidth: .infinity, alignment: .leading)

            self.primaryButton
        }
        .padding(6)
        .bobeRaised(in: self.shape, elevation: .lifted)
        .overlay {
            if self.isWorking {
                BobeWorkingRim(shape: self.shape)
                    .transition(.opacity)
            } else {
                self.shape
                    .strokeBorder(self.theme.colors.primary.opacity(self.isFocused ? 0.4 : 0), lineWidth: 1.5)
                    .allowsHitTesting(false)
            }
        }
        .animation(BobeMotion.fade, value: self.isWorking)
        .animation(BobeMotion.fade, value: self.isFocused)
        .contentShape(self.shape)
        .simultaneousGesture(TapGesture().onEnded { self.isFocused = true })
        .onAppear { self.isFocused = true }
    }

    @ViewBuilder
    private var primaryButton: some View {
        if self.showsStop {
            Button(action: self.stop) {
                Image(systemName: "stop.fill")
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(self.theme.colors.surface)
                    .frame(width: 34, height: 34)
                    .background(Circle().fill(self.theme.colors.text))
            }
            .buttonStyle(BobePressStyle(scale: 0.9))
            .disabled(self.runs.activeRun != nil && !self.runs.canStop && !self.canStopVoice)
            .accessibilityLabel(L10n.tr("overlay.run.stop"))
            .help(L10n.tr("overlay.run.stop"))
            .transition(.bobePop())
        } else {
            Button(action: self.submit) {
                Image(systemName: self.isSteering ? "arrow.turn.down.right" : "arrow.up")
                    .font(.system(size: 13, weight: .bold))
                    .contentTransition(.symbolEffect(.replace))
                    .foregroundStyle(self.hasText ? self.theme.onAccent : self.theme.colors.textMuted.opacity(0.6))
                    .frame(width: 34, height: 34)
                    .background(
                        Circle().fill(self.hasText ? self.theme.colors.primary : self.theme.well)
                    )
            }
            .buttonStyle(BobePressStyle(scale: 0.9))
            .disabled(!self.hasText || self.runs.isWorking)
            .accessibilityLabel(
                self.isSteering ? L10n.tr("overlay.run.guide") : L10n.tr("overlay.input.send.accessibility")
            )
            .accessibilityIdentifier("overlay.composer.send")
            .animation(BobeMotion.snappy, value: self.hasText)
            .transition(.bobePop())
        }
    }

    private func note(_ message: String, isError: Bool) -> some View {
        HStack(spacing: 6) {
            Image(systemName: isError ? "exclamationmark.triangle.fill" : "hourglass")
                .font(.system(size: 10, weight: .semibold))
                .foregroundStyle(isError ? self.theme.colors.error : self.theme.colors.primary)
            Text(message)
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.textMuted)
                .lineLimit(2)
        }
        .padding(.horizontal, 11)
        .padding(.vertical, 6)
        .bobeRaised(in: .bobePill)
    }

    private func submit() {
        guard self.hasText else { return }
        let trimmed = self.text.trimmingCharacters(in: .whitespacesAndNewlines)
        if self.runs.activeRun != nil {
            Task {
                guard await self.runs.guide(self.selection, text: trimmed) else { return }
                if self.text.trimmingCharacters(in: .whitespacesAndNewlines) == trimmed { self.text = "" }
            }
            return
        }
        guard self.onSend(trimmed) else { return }
        self.text = ""
    }

    private func stop() {
        if self.canStopVoice {
            self.onStopVoice()
        }
        if self.runs.activeRun != nil {
            Task { await self.runs.stop(self.selection) }
        }
    }
}

struct SteerModeChooser: View {
    @Binding var appliesLater: Bool
    @Environment(\.theme) private var theme
    @Namespace private var thumb

    var body: some View {
        HStack(spacing: 2) {
            self.option(
                isSelected: !self.appliesLater,
                systemName: "arrow.turn.down.right",
                title: L10n.tr("overlay.run.guide")
            ) { self.appliesLater = false }
            self.option(
                isSelected: self.appliesLater,
                systemName: "clock.arrow.circlepath",
                title: L10n.tr("overlay.run.follow_up")
            ) { self.appliesLater = true }
        }
        .padding(2)
        .bobeWell(in: .bobePill)
        .accessibilityElement(children: .contain)
    }

    private func option(
        isSelected: Bool,
        systemName: String,
        title: String,
        select: @escaping () -> Void
    ) -> some View {
        Button {
            withAnimation(BobeMotion.snappy) { select() }
        } label: {
            HStack(spacing: 5) {
                Image(systemName: systemName)
                    .font(.system(size: 11, weight: .semibold))
                if isSelected {
                    Text(title)
                        .font(.system(size: 12, weight: .medium))
                        .lineLimit(1)
                        .fixedSize()
                        .transition(.opacity.combined(with: .scale(scale: 0.9, anchor: .leading)))
                }
            }
            .foregroundStyle(isSelected ? self.theme.colors.text : self.theme.colors.textMuted)
            .padding(.horizontal, 10)
            .frame(height: 26)
            .background {
                if isSelected {
                    Capsule()
                        .fill(self.theme.colors.surface)
                        .shadow(color: self.theme.shadowTint.opacity(0.12), radius: 2, y: 1)
                        .matchedGeometryEffect(id: "thumb", in: self.thumb)
                }
            }
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .help(title)
        .accessibilityLabel(title)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}
