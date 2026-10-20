import SwiftUI
import UIKit

struct HomeView: View {
    @Bindable var model: PhoneModel

    var body: some View {
        NavigationStack {
            ZStack {
                PhoneBackdrop(glowCenter: UnitPoint(x: 0.5, y: 0.05))
                if self.model.thread.isEmpty, self.model.questions.isEmpty {
                    EmptyPresence(model: self.model)
                } else {
                    ThreadView(model: self.model)
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                ComposerBar(model: self.model)
            }
            .toolbar {
                ToolbarItem(placement: .principal) {
                    PresenceTitle(model: self.model)
                }
                ToolbarItem(placement: .topBarTrailing) {
                    SettingsButton(model: self.model)
                }
            }
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

private struct EmptyPresence: View {
    let model: PhoneModel
    @Environment(\.theme) private var theme

    private var greeting: String {
        switch Calendar.current.component(.hour, from: .now) {
        case 5 ..< 12: L10n.tr("phone.home.greeting.morning")
        case 12 ..< 18: L10n.tr("phone.home.greeting.afternoon")
        default: L10n.tr("phone.home.greeting.evening")
        }
    }

    private let suggestions = [
        "phone.home.suggestion.working",
        "phone.home.suggestion.remember",
        "phone.home.suggestion.week",
    ]

    var body: some View {
        ScrollView {
            VStack(spacing: 16) {
                if self.model.needsAttention {
                    ConnectionCard(model: self.model)
                }
                BobeCharacter(mood: self.model.mood, size: 150, palette: self.theme.avatar)
                    .padding(.top, 24)
                Text(self.greeting)
                    .phoneText(.title)
                    .foregroundStyle(self.theme.colors.text)
                    .multilineTextAlignment(.center)
                if self.model.link == .online {
                    VStack(spacing: 8) {
                        ForEach(self.suggestions, id: \.self) { key in
                            Button {
                                self.model.send(L10n.tr(key))
                            } label: {
                                Text(L10n.tr(key))
                                    .phoneText(.callout)
                                    .foregroundStyle(self.theme.colors.text)
                                    .padding(.horizontal, 16)
                                    .padding(.vertical, 11)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .bobeRaised(cornerRadius: BobeRadius.field)
                            }
                            .buttonStyle(BobePressStyle(scale: 0.98))
                        }
                    }
                    .padding(.top, 12)
                    .transition(.bobeRise)
                }
            }
            .padding(.horizontal, 20)
            .padding(.bottom, 24)
            .animation(BobeMotion.standard, value: self.model.link)
        }
        .scrollDismissesKeyboard(.interactively)
    }
}

struct ThreadView: View {
    let model: PhoneModel
    @Environment(\.theme) private var theme

    var body: some View {
        // swiftlint:disable:next accessibility_trait_for_button
        ScrollView {
            LazyVStack(spacing: 12) {
                if self.model.needsAttention {
                    ConnectionCard(model: self.model)
                }
                ForEach(self.model.questions) { question in
                    QuestionCard(model: self.model, question: question)
                }
                ForEach(self.model.thread) { message in
                    MessageRow(model: self.model, message: message)
                }
                if self.model.showsTyping {
                    TypingRow(text: self.model.status)
                        .transition(.bobeRise)
                }
                if self.model.replyTrouble {
                    Label(L10n.tr("phone.thread.trouble"), systemImage: "exclamationmark.bubble")
                        .phoneText(.footnote)
                        .foregroundStyle(self.theme.colors.warning)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 10)
            .padding(.bottom, 14)
            .animation(BobeMotion.standard, value: self.model.showsTyping)
        }
        .defaultScrollAnchor(.bottom)
        .defaultScrollAnchor(.bottom, for: .sizeChanges)
        .scrollDismissesKeyboard(.immediately)
        .onTapGesture {
            UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
        }
    }
}

struct MessageRow: View {
    let model: PhoneModel
    let message: ThreadMessage
    @State private var visible = false
    @Environment(\.theme) private var theme
    @Environment(\.scenePhase) private var scenePhase

    private var isMine: Bool {
        self.message.author == .me
    }

    private var presentationKey: String {
        [
            self.message.id,
            "\(self.visible)",
            "\(self.message.isComplete)",
            "\(self.message.isStreaming)",
            "\(self.scenePhase == .active)",
        ].joined(separator: ":")
    }

    var body: some View {
        HStack(alignment: .bottom, spacing: 0) {
            if self.isMine { Spacer(minLength: 52) }
            VStack(alignment: self.isMine ? .trailing : .leading, spacing: 5) {
                if self.message.reachedOut {
                    Label(L10n.tr("phone.thread.reached_out"), systemImage: "sparkles")
                        .phoneText(.label)
                        .foregroundStyle(self.theme.colors.primary)
                }
                self.bubble
                if self.message.delivery == .failed {
                    Button {
                        self.model.retry(self.message)
                    } label: {
                        Label(L10n.tr("phone.thread.retry"), systemImage: "exclamationmark.circle.fill")
                            .phoneText(.footnote)
                    }
                    .foregroundStyle(self.theme.colors.warning)
                } else if self.message.delivery == .unconfirmed {
                    Text(L10n.tr("phone.thread.unconfirmed"))
                        .phoneText(.footnote)
                        .foregroundStyle(self.theme.colors.textMuted)
                        .multilineTextAlignment(.trailing)
                        .fixedSize(horizontal: false, vertical: true)
                    Button {
                        self.model.sendAgain(self.message)
                    } label: {
                        Label(L10n.tr("phone.thread.send_again"), systemImage: "arrow.clockwise")
                            .phoneText(.footnote)
                    }
                    .foregroundStyle(self.theme.colors.warning)
                }
            }
            if !self.isMine { Spacer(minLength: 36) }
        }
        .onGeometryChange(for: Bool.self) { geometry in
            guard let viewport = geometry.bounds(of: .scrollView) else { return false }
            let row = geometry.frame(in: .local)
            let visible = row.intersection(viewport)
            return MessagePresentation.isPresented(
                visibleHeight: visible.isNull ? 0 : visible.height,
                rowHeight: row.height,
                viewportHeight: viewport.height
            )
        } action: { self.visible = $0 }
        .task(id: self.presentationKey) {
            if self.visible, self.scenePhase == .active, !self.isMine, self.message.isComplete, !self.message.isStreaming {
                self.model.presented(self.message)
            }
        }
    }

    @ViewBuilder
    private var bubble: some View {
        let shape = RoundedRectangle(cornerRadius: 20, style: .continuous)
        if self.isMine {
            Text(self.message.text)
                .phoneText(.body)
                .foregroundStyle(self.theme.colors.text)
                .padding(.horizontal, 15)
                .padding(.vertical, 10)
                .background(self.theme.accentSoft, in: shape)
                .opacity(self.message.delivery == .sending ? 0.65 : 1)
                .textSelection(.enabled)
        } else {
            HStack(alignment: .lastTextBaseline, spacing: 6) {
                Text(Self.rendered(self.message.text))
                    .phoneText(.body)
                    .foregroundStyle(self.theme.colors.text)
                    .textSelection(.enabled)
                if self.message.isStreaming {
                    BobeBreathingDot(size: 7)
                }
            }
            .padding(.horizontal, 15)
            .padding(.vertical, 11)
            .bobeRaised(in: shape)
        }
    }

    private static func rendered(_ text: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }
}

nonisolated enum MessagePresentation {
    static func isPresented(visibleHeight: CGFloat, rowHeight: CGFloat, viewportHeight: CGFloat) -> Bool {
        let required = min(rowHeight, viewportHeight) / 2
        return required > 0 && visibleHeight >= required
    }
}

private struct TypingRow: View {
    let text: String

    var body: some View {
        HStack {
            BobeShimmerText(text: self.text, font: PhoneText.callout.font)
            .padding(.horizontal, 15)
            .padding(.vertical, 11)
            .bobeRaised(in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            Spacer(minLength: 36)
        }
    }
}

struct ConnectionCard: View {
    let model: PhoneModel
    @Environment(\.theme) private var theme

    private var removed: Bool {
        self.model.link == .removed
    }

    private var symbol: String {
        switch self.model.link {
        case .removed: "iphone.slash"
        case .incompatible: "arrow.triangle.2.circlepath"
        case .settingUp: "hourglass"
        default: "wifi.exclamationmark"
        }
    }

    private var title: String {
        switch self.model.link {
        case .removed: L10n.tr("phone.connection.removed.title")
        case .incompatible: L10n.tr("phone.connection.incompatible.title")
        case .settingUp: L10n.tr("phone.connection.setting_up.title")
        default: L10n.tr("phone.connection.offline.title")
        }
    }

    private var detail: String? {
        switch self.model.link {
        case .incompatible: L10n.tr("phone.connection.incompatible.body")
        case .settingUp: L10n.tr("phone.connection.setting_up.body")
        default: nil
        }
    }

    var body: some View {
        PhoneCard {
            HStack(alignment: .top, spacing: 12) {
                BobeIconTile(
                    systemName: self.symbol,
                    tint: self.theme.colors.warning,
                    size: 32
                )
                VStack(alignment: .leading, spacing: 8) {
                    Text(self.title)
                        .phoneText(.heading)
                        .foregroundStyle(self.theme.colors.text)
                    if let detail {
                        Text(detail)
                            .phoneText(.callout)
                            .foregroundStyle(self.theme.colors.textMuted)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if case let .failed(message) = self.model.pairing {
                        Text(message)
                            .phoneText(.callout)
                            .foregroundStyle(self.theme.colors.textMuted)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if self.model.pairing == .working {
                        ProgressView()
                    } else {
                        Button(L10n.tr(self.removed ? "phone.connection.pair_again" : "phone.connection.retry")) {
                            if self.removed {
                                self.model.unpair()
                            } else {
                                self.model.reconnect()
                            }
                        }
                        .buttonStyle(.phoneSecondary)
                        .fixedSize()
                    }
                }
            }
        }
        .transition(.bobeRise)
    }
}
