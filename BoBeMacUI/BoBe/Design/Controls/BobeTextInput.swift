import SwiftUI

private struct BobeInputChromeModifier: ViewModifier {
    let focused: Bool
    let hovered: Bool
    @Environment(\.theme) private var theme

    func body(content: Content) -> some View {
        let shape = RoundedRectangle(cornerRadius: BobeRadius.control, style: .continuous)
        content
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .bobeWell(in: shape)
            .overlay {
                shape
                    .strokeBorder(
                        self.theme.colors.primary.opacity(self.focused ? 0.75 : (self.hovered ? 0.25 : 0)),
                        lineWidth: 1.5
                    )
            }
            .background {
                shape
                    .stroke(self.theme.colors.primary.opacity(self.focused ? 0.2 : 0), lineWidth: 5)
            }
            .animation(BobeMotion.fade, value: self.focused)
            .animation(BobeMotion.fade, value: self.hovered)
    }
}

private struct BobeFieldModifier: ViewModifier {
    @FocusState private var isFocused: Bool
    @State private var isHovered = false

    func body(content: Content) -> some View {
        content
            .textFieldStyle(.plain)
            .font(.system(size: 13))
            .focused(self.$isFocused)
            .bobeInputChrome(focused: self.isFocused, hovered: self.isHovered)
            .onHover { self.isHovered = $0 }
    }
}

extension View {
    func bobeInputChrome(focused: Bool, hovered: Bool) -> some View {
        modifier(BobeInputChromeModifier(focused: focused, hovered: hovered))
    }

    func bobeField() -> some View {
        modifier(BobeFieldModifier())
    }
}

struct BobeTextField: View {
    let placeholder: String
    @Binding var text: String
    var width: CGFloat?
    var alignment: TextAlignment
    var focused: Binding<Bool>?
    var onSubmit: (() -> Void)?

    init(
        placeholder: String,
        text: Binding<String>,
        width: CGFloat? = nil,
        alignment: TextAlignment = .leading,
        focused: Binding<Bool>? = nil,
        onSubmit: (() -> Void)? = nil
    ) {
        self.placeholder = placeholder
        _text = text
        self.width = width
        self.alignment = alignment
        self.focused = focused
        self.onSubmit = onSubmit
    }

    @Environment(\.theme) private var theme
    @FocusState private var isFocused: Bool
    @State private var isHovered = false

    var body: some View {
        TextField("", text: self.$text)
        .accessibilityLabel(self.placeholder)
        .textFieldStyle(.plain)
        .font(.system(size: 13))
        .foregroundStyle(self.theme.colors.text)
        .multilineTextAlignment(self.alignment)
        .tint(self.theme.colors.primary)
        .focused(self.$isFocused)
        .onChange(of: self.focused?.wrappedValue) { _, requested in
            guard let requested else { return }
            self.isFocused = requested
        }
        .onChange(of: self.isFocused) { _, focused in
            if self.focused?.wrappedValue != focused {
                self.focused?.wrappedValue = focused
            }
        }
        .task {
            if self.focused?.wrappedValue == true {
                self.isFocused = true
            }
        }
        .background(alignment: self.alignment == .trailing ? .trailing : .leading) {
            BobePlaceholder(text: self.placeholder, isVisible: self.text.isEmpty)
        }
        .bobeInputChrome(focused: self.isFocused, hovered: self.isHovered)
        .onHover { self.isHovered = $0 }
        .onSubmit { self.onSubmit?() }
        .frame(width: self.width)
    }
}

struct BobePlaceholder: View {
    let text: String
    let isVisible: Bool
    @Environment(\.theme) private var theme

    var body: some View {
        if self.isVisible {
            Text(self.text)
                .font(.system(size: 13))
                .foregroundStyle(self.theme.colors.textMuted.opacity(0.75))
                .lineLimit(1)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
    }
}

struct BobeSecureField: View {
    let placeholder: String
    @Binding var text: String
    var width: CGFloat?
    var onSubmit: (() -> Void)?

    init(
        placeholder: String,
        text: Binding<String>,
        width: CGFloat? = nil,
        onSubmit: (() -> Void)? = nil
    ) {
        self.placeholder = placeholder
        _text = text
        self.width = width
        self.onSubmit = onSubmit
    }

    @Environment(\.theme) private var theme
    @FocusState private var isFocused: Bool
    @State private var isHovered = false

    var body: some View {
        SecureField("", text: self.$text)
        .accessibilityLabel(self.placeholder)
        .textFieldStyle(.plain)
        .font(.system(size: 13))
        .foregroundStyle(self.theme.colors.text)
        .tint(self.theme.colors.primary)
        .focused(self.$isFocused)
        .background(alignment: .leading) {
            BobePlaceholder(text: self.placeholder, isVisible: self.text.isEmpty)
        }
        .bobeInputChrome(focused: self.isFocused, hovered: self.isHovered)
        .onHover { self.isHovered = $0 }
        .onSubmit { self.onSubmit?() }
        .frame(width: self.width)
    }
}
