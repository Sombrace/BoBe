import SwiftUI

struct BobeSwitchStyle: ToggleStyle {
    @Environment(\.theme) private var theme
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        Button {
            withAnimation(BobeMotion.snappy) {
                configuration.isOn.toggle()
            }
        } label: {
            Capsule(style: .circular)
                .fill(configuration.isOn ? self.theme.colors.primary : self.theme.colors.textMuted.opacity(0.3))
                .frame(width: 38, height: 22)
                .overlay(alignment: configuration.isOn ? .trailing : .leading) {
                    Circle()
                        .fill(.white)
                        .shadow(color: .black.opacity(0.22), radius: 1.5, y: 1)
                        .frame(width: 18, height: 18)
                        .padding(2)
                }
                .opacity(self.isEnabled ? 1 : 0.5)
                .contentShape(Capsule(style: .circular))
        }
        .buttonStyle(BobePressStyle(scale: 0.94))
        .accessibilityRepresentation {
            Toggle(isOn: configuration.$isOn) { configuration.label }
        }
    }
}

struct BobeLabeledSwitchStyle: ToggleStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 12) {
            configuration.label
                .frame(maxWidth: .infinity, alignment: .leading)
            Toggle(isOn: configuration.$isOn) { EmptyView() }
                .labelsHidden()
                .toggleStyle(BobeSwitchStyle())
        }
        .accessibilityRepresentation {
            Toggle(isOn: configuration.$isOn) { configuration.label }
        }
    }
}

struct BobeHairline: View {
    @Environment(\.theme) private var theme

    var body: some View {
        Rectangle()
            .fill(self.theme.hairline)
            .frame(height: 1)
            .accessibilityHidden(true)
    }
}

struct BobeDisclosure<Content: View>: View {
    let title: String
    private let external: Binding<Bool>?
    private let content: Content

    @State private var internalExpanded = false
    @Environment(\.theme) private var theme

    init(_ title: String, isExpanded: Binding<Bool>? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.external = isExpanded
        self.content = content()
    }

    private var isExpanded: Bool {
        self.external?.wrappedValue ?? self.internalExpanded
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                withAnimation(BobeMotion.standard) {
                    if let external {
                        external.wrappedValue.toggle()
                    } else {
                        self.internalExpanded.toggle()
                    }
                }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 9, weight: .bold))
                        .rotationEffect(.degrees(self.isExpanded ? 90 : 0))
                    Text(self.title)
                        .bobeTextStyle(.helper)
                    Spacer(minLength: 0)
                }
                .foregroundStyle(self.theme.colors.textMuted)
                .padding(.vertical, 4)
                .contentShape(Rectangle())
            }
            .buttonStyle(BobePressStyle(scale: 0.99))
            .accessibilityValue(self.isExpanded ? "expanded" : "collapsed")
            if self.isExpanded {
                self.content
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .transition(.bobeRise)
            }
        }
    }
}

struct BobeToggle: View {
    @Binding var isOn: Bool
    let accessibilityLabel: String

    init(isOn: Binding<Bool>, accessibilityLabel: String) {
        self._isOn = isOn
        self.accessibilityLabel = accessibilityLabel
    }

    var body: some View {
        Toggle(self.accessibilityLabel, isOn: self.$isOn)
            .labelsHidden()
            .toggleStyle(BobeSwitchStyle())
    }
}

struct SettingsSectionHeader: View {
    let icon: String
    let title: String
    var description: String?
    var tint: Color?
    @Environment(\.theme) private var theme

    var body: some View {
        HStack(spacing: 11) {
            BobeIconTile(systemName: self.icon, tint: self.tint ?? self.theme.colors.primary, size: 28)
            VStack(alignment: .leading, spacing: 1) {
                Text(self.title)
                    .bobeTextStyle(.heading)
                    .foregroundStyle(self.theme.colors.text)
                if let description {
                    Text(description)
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 0)
        }
    }
}

struct SettingsErrorBanner: View {
    let message: String
    @Environment(\.theme) private var theme

    var body: some View {
        HStack(spacing: 10) {
            BobeIconTile(systemName: "exclamationmark", tint: self.theme.colors.error, size: 26)
            Text(self.message)
                .bobeTextStyle(.body)
                .foregroundStyle(self.theme.colors.text)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .bobeRaised(cornerRadius: 18)
        .transition(.bobeRise)
    }
}

struct SettingsSavedToast: View {
    let message: String
    @Environment(\.theme) private var theme

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(self.theme.colors.success)
            Text(self.message)
                .bobeTextStyle(.helper)
                .fontWeight(.medium)
                .foregroundStyle(self.theme.colors.success)
            Spacer()
        }
        .transition(.bobePop(anchor: .leading))
    }
}

struct AccentAddButton: View {
    var title = L10n.tr("settings.shared.action.add_new")
    let action: () -> Void

    var body: some View {
        Button(self.title, action: self.action)
            .bobeButton(.primary, size: .small)
    }
}

struct SettingsPaneHeader: View {
    let title: String
    var actionTitle = L10n.tr("settings.shared.action.add_new")
    let action: () -> Void
    @Environment(\.theme) private var theme

    var body: some View {
        HStack {
            Text(self.title)
                .bobeTextStyle(.title)
                .foregroundStyle(self.theme.colors.text)
            Spacer()
            AccentAddButton(title: self.actionTitle, action: self.action)
        }
    }
}

struct SettingsRow<Content: View>: View {
    let label: String
    var description: String?
    var suffix: String?
    @ViewBuilder let content: Content
    @Environment(\.theme) private var theme

    var body: some View {
        HStack(alignment: .center, spacing: 16) {
            VStack(alignment: .leading, spacing: 2) {
                Text(self.label)
                    .bobeTextStyle(.rowTitle)
                    .foregroundStyle(self.theme.colors.text)
                    .fixedSize(horizontal: false, vertical: true)
                if let description {
                    Text(description)
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            HStack(alignment: .center, spacing: 8) {
                self.content
                if let suffix {
                    Text(suffix)
                        .bobeTextStyle(.helper)
                        .foregroundStyle(self.theme.colors.textMuted)
                        .fixedSize(horizontal: true, vertical: false)
                }
            }
            .fixedSize(horizontal: true, vertical: false)
        }
        .frame(minHeight: 34)
    }
}

struct ThemedSplitPane<Left: View, Right: View>: View {
    let leftWidth: CGFloat
    @ViewBuilder let left: Left
    @ViewBuilder let right: Right
    @Environment(\.theme) private var theme

    var body: some View {
        HStack(spacing: 0) {
            self.left
                .frame(width: self.leftWidth)
            Rectangle()
                .fill(self.theme.hairline)
                .frame(width: 1)
                .padding(.vertical, 14)
            self.right
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

struct DebouncedNumberInput: View {
    @Binding var value: Int
    var range: ClosedRange<Int> = 0 ... 9999
    var width: CGFloat = 80

    @State private var text = ""
    @State private var debounceTask: Task<Void, Never>?
    @FocusState private var isFocused: Bool
    @State private var isHovered = false
    @Environment(\.theme) private var theme

    var body: some View {
        TextField("", text: self.$text)
            .textFieldStyle(.plain)
            .font(.system(size: 13, weight: .medium).monospacedDigit())
            .multilineTextAlignment(.trailing)
            .foregroundStyle(self.theme.colors.text)
            .tint(self.theme.colors.primary)
            .focused(self.$isFocused)
            .bobeInputChrome(focused: self.isFocused, hovered: self.isHovered)
            .onHover { self.isHovered = $0 }
            .frame(width: self.width)
            .onChange(of: self.text) { _, newText in
                guard self.isFocused else { return }
                self.debounceTask?.cancel()
                self.debounceTask = Task { @MainActor in
                    try? await Task.sleep(for: .seconds(0.6))
                    if let parsed = Int(newText), range.contains(parsed) {
                        self.value = parsed
                    }
                }
            }
            .onSubmit {
                self.debounceTask?.cancel()
                if let parsed = Int(text), range.contains(parsed) {
                    self.value = parsed
                }
            }
            .onAppear { self.text = String(self.value) }
            .onChange(of: self.value) { _, newVal in
                guard !self.isFocused else { return }
                let str = String(newVal)
                if self.text != str { self.text = str }
            }
    }
}

struct CollapsibleSection<Content: View>: View {
    let title: String
    let icon: String
    var description: String?
    var toggleBinding: Binding<Bool>?
    var aiCuratedBadge = false
    var initiallyExpanded = true
    @ViewBuilder let content: Content

    @State private var isExpanded: Bool
    @State private var isHeaderHovered = false
    @Environment(\.theme) private var theme

    init(
        title: String,
        icon: String,
        description: String? = nil,
        toggleBinding: Binding<Bool>? = nil,
        aiCuratedBadge: Bool = false,
        initiallyExpanded: Bool = true,
        @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.icon = icon
        self.description = description
        self.toggleBinding = toggleBinding
        self.aiCuratedBadge = aiCuratedBadge
        self.initiallyExpanded = initiallyExpanded
        self.content = content()
        self._isExpanded = State(initialValue: initiallyExpanded)
    }

    private var isOff: Bool {
        self.toggleBinding.map { !$0.wrappedValue } ?? false
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            self.header
            if self.isExpanded {
                VStack(alignment: .leading, spacing: 12) {
                    self.content
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 16)
                .padding(.top, 2)
                .disabled(self.isOff)
                .opacity(self.isOff ? 0.45 : 1)
                .transition(.bobeRise)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .bobeRaised(cornerRadius: BobeRadius.card)
        .animation(BobeMotion.standard, value: self.isExpanded)
        .animation(BobeMotion.fade, value: self.isOff)
    }

    private var header: some View {
        HStack(spacing: 11) {
            Button {
                self.isExpanded.toggle()
            } label: {
                HStack(spacing: 11) {
                    BobeIconTile(
                        systemName: self.icon,
                        tint: self.isOff ? self.theme.colors.textMuted : self.theme.colors.primary,
                        size: 28
                    )
                    VStack(alignment: .leading, spacing: 1) {
                        HStack(spacing: 6) {
                            Text(self.title)
                                .bobeTextStyle(.heading)
                                .foregroundStyle(self.theme.colors.text)
                            if self.aiCuratedBadge {
                                Text(L10n.tr("settings.goals.section.ai_curated_badge"))
                                    .bobeTextStyle(.badge)
                                    .foregroundStyle(self.theme.colors.primary)
                                    .padding(.horizontal, 7)
                                    .frame(height: 18)
                                    .background(Capsule(style: .circular).fill(self.theme.accentSoft))
                            }
                        }
                        if let description {
                            Text(description)
                                .bobeTextStyle(.helper)
                                .foregroundStyle(self.theme.colors.textMuted)
                                .multilineTextAlignment(.leading)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    Spacer(minLength: 8)
                    Image(systemName: "chevron.right")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(self.theme.colors.textMuted.opacity(self.isHeaderHovered ? 1 : 0.6))
                        .rotationEffect(.degrees(self.isExpanded ? 90 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityValue(self.isExpanded ? L10n.tr("settings.providers.on") : L10n.tr("settings.providers.off"))

            if let toggleBinding {
                BobeToggle(
                    isOn: toggleBinding,
                    accessibilityLabel: L10n.tr("settings.shared.section.enabled_format", self.title)
                )
            }
        }
        .padding(14)
        .onHover { self.isHeaderHovered = $0 }
    }
}

#if !SPM_BUILD
    #Preview("BobeToggle") {
        @Previewable @State var isOn = true
        HStack(spacing: 20) {
            BobeToggle(isOn: $isOn, accessibilityLabel: "Example toggle")
            Text(isOn ? "On" : "Off")
        }
        .environment(\.theme, allThemes[0])
        .padding()
    }

    #Preview("SettingsRow") {
        @Previewable @State var toggle = true
        VStack(spacing: 16) {
            SettingsRow(label: "Enable Feature", description: "A helpful description") {
                BobeToggle(isOn: $toggle, accessibilityLabel: "Enable feature")
            }
            SettingsRow(label: "Token Limit", suffix: "tokens") {
                Text("4096")
                    .font(.system(size: 13, design: .monospaced))
            }
        }
        .environment(\.theme, allThemes[0])
        .padding()
        .frame(width: 400)
    }

    #Preview("CollapsibleSection") {
        @Previewable @State var toggle = true
        CollapsibleSection(title: "Screen Capture", icon: "camera.fill", description: "Periodic screenshots", toggleBinding: $toggle) {
            Text("Section content goes here")
        }
        .environment(\.theme, allThemes[0])
        .padding()
        .frame(width: 400)
    }

    #Preview("DebouncedNumberInput") {
        @Previewable @State var value = 4096
        SettingsRow(label: "Max Tokens") {
            DebouncedNumberInput(value: $value, range: 1 ... 8192)
        }
        .environment(\.theme, allThemes[0])
        .padding()
        .frame(width: 400)
    }
#endif
