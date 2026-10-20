import SwiftUI
import UIKit

struct ComposerBar: View {
    @Bindable var model: PhoneModel
    @State private var cancelArmed = false
    @FocusState private var focused: Bool
    @Environment(\.theme) private var theme

    private var canSend: Bool {
        self.model.link == .online
            && !self.model.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    private var placeholder: String {
        switch self.model.link {
        case .offline: L10n.tr("phone.composer.offline")
        case .settingUp: L10n.tr("phone.connection.setting_up.title")
        default: L10n.tr("phone.composer.placeholder")
        }
    }

    var body: some View {
        VStack(spacing: 8) {
            if self.model.talk.state == .listening {
                ListeningCard(talk: self.model.talk, cancelArmed: self.cancelArmed)
                    .transition(.bobeRise)
            } else if self.model.talk.state == .preparing {
                self.preparingNote
                    .transition(.bobeRise)
            } else if self.model.talk.state == .unavailable {
                self.unavailableNote
                    .transition(.bobeRise)
            }
            HStack(alignment: .bottom, spacing: 10) {
                TextField(
                    self.placeholder,
                    text: self.$model.draft,
                    axis: .vertical
                )
                .lineLimit(1 ... 5)
                .focused(self.$focused)
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .frame(minHeight: 48)
                .glassEffect(.regular, in: .rect(cornerRadius: 24))
                .accessibilityIdentifier("phone.composer.field")
                if self.model.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    TalkButton(model: self.model, cancelArmed: self.$cancelArmed)
                        .transition(.bobePop())
                } else {
                    Button {
                        self.model.send()
                    } label: {
                        Image(systemName: "arrow.up")
                            .font(.system(size: 19, weight: .bold))
                            .foregroundStyle(self.theme.onAccent)
                            .frame(width: 48, height: 48)
                            .background(self.theme.colors.primary, in: Circle())
                    }
                    .buttonStyle(BobePressStyle())
                    .disabled(!self.canSend)
                    .opacity(self.canSend ? 1 : 0.45)
                    .accessibilityLabel(L10n.tr("phone.composer.send"))
                    .accessibilityIdentifier("phone.composer.send")
                    .transition(.bobePop())
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.top, 6)
        .padding(.bottom, 8)
        .animation(BobeMotion.snappy, value: self.model.draft.isEmpty)
        .animation(BobeMotion.standard, value: self.model.talk.state)
    }

    private var preparingNote: some View {
        HStack(spacing: 10) {
            ProgressView()
                .controlSize(.small)
            Text(L10n.tr("phone.talk.preparing"))
                .phoneText(.footnote)
                .foregroundStyle(self.theme.colors.text)
            Spacer(minLength: 0)
        }
        .padding(12)
        .bobeRaised(cornerRadius: BobeRadius.control)
    }

    private var unavailableNote: some View {
        HStack(spacing: 10) {
            Image(systemName: "mic.slash.fill")
                .foregroundStyle(self.theme.colors.warning)
            Text(L10n.tr(PhonePermissions.voice() == .granted ? "phone.talk.unavailable" : "phone.talk.permission"))
                .phoneText(.footnote)
                .foregroundStyle(self.theme.colors.text)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            Button {
                self.model.talk.dismissUnavailable()
            } label: {
                Image(systemName: "xmark")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(self.theme.colors.textMuted)
            }
            .accessibilityLabel(L10n.tr("phone.common.dismiss"))
        }
        .padding(12)
        .bobeRaised(cornerRadius: BobeRadius.control)
    }
}

private struct TalkButton: View {
    let model: PhoneModel
    @Binding var cancelArmed: Bool
    @State private var holding = false
    @Environment(\.theme) private var theme

    private var enabled: Bool {
        self.model.link == .online
    }

    var body: some View {
        ZStack {
            Circle()
                .fill(self.holding ? self.theme.colors.primary : self.theme.colors.surface)
            Circle()
                .strokeBorder(self.theme.glow[1].opacity(0.55), lineWidth: 3)
                .scaleEffect(1 + self.model.talk.level * 0.45)
                .opacity(self.holding ? 1 : 0)
            Image(systemName: self.cancelArmed ? "xmark" : "mic.fill")
                .font(.system(size: 19, weight: .semibold))
                .foregroundStyle(self.holding ? self.theme.onAccent : self.theme.colors.primary)
        }
        .frame(width: 48, height: 48)
        .overlay { Circle().strokeBorder(self.theme.hairline, lineWidth: self.holding ? 0 : 1) }
        .scaleEffect(self.holding ? 1.12 : 1)
        .animation(BobeMotion.snappy, value: self.holding)
        .sensoryFeedback(.impact(weight: .medium), trigger: self.holding) { _, holding in holding }
        .opacity(self.enabled ? 1 : 0.45)
        .gesture(
            DragGesture(minimumDistance: 0)
                .onChanged { value in
                    guard self.enabled else { return }
                    if !self.holding {
                        self.holding = true
                        self.model.talk.start()
                    }
                    self.cancelArmed = value.translation.height < -60
                }
                .onEnded { _ in
                    guard self.holding else { return }
                    self.holding = false
                    if self.cancelArmed {
                        self.model.talk.cancel()
                    } else {
                        self.release()
                    }
                    self.cancelArmed = false
                }
        )
        .accessibilityElement()
        .accessibilityLabel(L10n.tr("phone.talk.label"))
        .accessibilityHint(L10n.tr("phone.talk.hint"))
        .accessibilityAddTraits(.isButton)
        .accessibilityAction {
            guard self.enabled else { return }
            if self.model.talk.state == .listening {
                self.release()
            } else {
                self.model.talk.start()
            }
        }
        .accessibilityIdentifier("phone.composer.talk")
    }

    private func release() {
        Task {
            let text = await self.model.talk.finish()
            if !text.isEmpty {
                self.model.send(text)
            }
        }
    }
}

private struct ListeningCard: View {
    let talk: HoldToTalk
    let cancelArmed: Bool
    @Environment(\.theme) private var theme

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                BobeBreathingDot(size: 8)
                Text(L10n.tr(self.cancelArmed ? "phone.talk.release_cancel" : "phone.talk.listening"))
                    .phoneText(.label)
                    .foregroundStyle(self.cancelArmed ? self.theme.colors.warning : self.theme.colors.primary)
                Spacer()
                Text(L10n.tr("phone.talk.slide_cancel"))
                    .phoneText(.footnote)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .opacity(self.cancelArmed ? 0 : 1)
            }
            Text(self.talk.transcript.isEmpty ? L10n.tr("phone.talk.speak_now") : self.talk.transcript)
                .phoneText(.body)
                .foregroundStyle(self.talk.transcript.isEmpty ? self.theme.colors.textMuted : self.theme.colors.text)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentTransition(.opacity)
        }
        .padding(14)
        .bobeRaised(cornerRadius: BobeRadius.card, elevation: .lifted)
        .animation(BobeMotion.fade, value: self.talk.transcript)
    }
}
