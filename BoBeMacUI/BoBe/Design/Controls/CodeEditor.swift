import AppKit
import SwiftUI

struct CodeEditor: NSViewRepresentable {
    @Binding var text: String
    let theme: ThemeConfig
    var fontSize: CGFloat = 13

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let scrollView = NSScrollView()
        scrollView.hasVerticalScroller = true
        scrollView.hasHorizontalScroller = false
        scrollView.autohidesScrollers = true
        scrollView.borderType = .noBorder
        scrollView.drawsBackground = true
        scrollView.backgroundColor = NSColor(self.theme.colors.surface)
        scrollView.wantsLayer = true
        scrollView.layer?.cornerRadius = 8
        scrollView.layer?.masksToBounds = true
        context.coordinator.scrollView = scrollView

        let textView = FocusAwareTextView()
        textView.delegate = context.coordinator
        textView.string = self.text
        textView.isEditable = true
        textView.isSelectable = true
        textView.isRichText = false
        textView.importsGraphics = false
        textView.usesFindBar = true
        textView.allowsUndo = true
        textView.isAutomaticQuoteSubstitutionEnabled = false
        textView.isAutomaticDataDetectionEnabled = false
        textView.isAutomaticDashSubstitutionEnabled = false
        textView.isAutomaticSpellingCorrectionEnabled = false
        textView.smartInsertDeleteEnabled = false
        textView.textContainerInset = NSSize(width: 12, height: 10)
        textView.isHorizontallyResizable = false
        textView.isVerticallyResizable = true
        textView.autoresizingMask = [.width]
        textView.textContainer?.widthTracksTextView = true
        textView.textContainer?.containerSize = NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude)
        textView.textContainer?.lineFragmentPadding = 4
        textView.focusRingType = .none
        self.applyTheme(to: textView)
        self.applyTypography(to: textView)
        textView.font = .monospacedSystemFont(ofSize: self.fontSize, weight: .regular)
        textView.onFocusChange = { isFocused in
            context.coordinator.isFocused = isFocused
            context.coordinator.updateFocusBorder(theme: self.theme)
        }
        context.coordinator.updateFocusBorder(theme: self.theme)

        scrollView.documentView = textView
        return scrollView
    }

    func updateNSView(_ scrollView: NSScrollView, context: Context) {
        context.coordinator.parent = self
        scrollView.backgroundColor = NSColor(self.theme.colors.surface)
        guard let textView = scrollView.documentView as? NSTextView else { return }
        if textView.string != self.text {
            let ranges = textView.selectedRanges
            textView.string = self.text
            let maxLen = (textView.string as NSString).length
            let clamped = ranges.compactMap { rangeValue -> NSValue? in
                let range = rangeValue.rangeValue
                guard range.location <= maxLen else { return nil }
                let clampedLength = min(range.length, maxLen - range.location)
                return NSValue(range: NSRange(location: range.location, length: clampedLength))
            }
            textView.selectedRanges = clamped.isEmpty ? [NSValue(range: NSRange(location: maxLen, length: 0))] : clamped
        }
        textView.font = .monospacedSystemFont(ofSize: self.fontSize, weight: .regular)
        self.applyTypography(to: textView)
        self.applyTheme(to: textView)
        context.coordinator.updateFocusBorder(theme: self.theme)
    }

    private func applyTypography(to textView: NSTextView) {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineSpacing = 2
        paragraph.paragraphSpacing = 3
        paragraph.defaultTabInterval = 20
        textView.defaultParagraphStyle = paragraph

        var attrs = textView.typingAttributes
        attrs[.paragraphStyle] = paragraph
        attrs[.font] = NSFont.monospacedSystemFont(ofSize: self.fontSize, weight: .regular)
        attrs[.foregroundColor] = NSColor(self.theme.colors.text)
        textView.typingAttributes = attrs
    }

    private func applyTheme(to textView: NSTextView) {
        textView.backgroundColor = NSColor(self.theme.colors.surface)
        textView.textColor = NSColor(self.theme.colors.text)
        textView.insertionPointColor = NSColor(self.theme.colors.primary)
        textView.selectedTextAttributes = [
            .backgroundColor: NSColor(self.theme.colors.primary).withAlphaComponent(0.3),
            .foregroundColor: NSColor(self.theme.colors.text),
        ]
    }

    @MainActor
    final class Coordinator: NSObject, NSTextViewDelegate {
        var parent: CodeEditor
        weak var scrollView: NSScrollView?
        var isFocused = false

        init(_ parent: CodeEditor) {
            self.parent = parent
        }

        func textDidChange(_ notification: Notification) {
            guard let textView = notification.object as? NSTextView else { return }
            let value = textView.string
            if self.parent.text != value {
                self.parent.text = value
            }
        }

        func updateFocusBorder(theme: ThemeConfig) {
            guard let layer = scrollView?.layer else { return }
            layer.borderWidth = self.isFocused ? 1.5 : 1
            layer.borderColor = (self.isFocused ? NSColor(theme.colors.primary) : NSColor(theme.colors.border)).cgColor
        }
    }
}

final class FocusAwareTextView: NSTextView {
    var onFocusChange: ((Bool) -> Void)?

    override func becomeFirstResponder() -> Bool {
        let didFocus = super.becomeFirstResponder()
        if didFocus { self.onFocusChange?(true) }
        return didFocus
    }

    override func resignFirstResponder() -> Bool {
        let didBlur = super.resignFirstResponder()
        if didBlur { self.onFocusChange?(false) }
        return didBlur
    }
}

#if !SPM_BUILD
    #Preview("Code Editor") {
        @Previewable @State var text = """
        You are BoBe, a friendly AI companion.
        Be helpful, proactive, and concise.
        Observe the user's workflow and offer suggestions.
        """
        CodeEditor(text: $text, theme: allThemes[0])
            .frame(width: 500, height: 300)
            .padding()
    }
#endif
