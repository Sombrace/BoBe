import AppKit
import SwiftUI

struct BobeMenuPicker<Option: Hashable>: View {
    @Binding var selection: Option
    let options: [Option]
    let label: (Option) -> String
    var width: CGFloat?
    var size: BobeControlSize

    init(
        selection: Binding<Option>,
        options: [Option],
        label: @escaping (Option) -> String,
        width: CGFloat? = nil,
        size: BobeControlSize = .regular
    ) {
        _selection = selection
        self.options = options
        self.label = label
        self.width = width
        self.size = size
    }

    @Environment(\.theme) private var theme

    var body: some View {
        ThemedPopUpButton(
            selection: self.$selection,
            options: self.options,
            label: self.label,
            isDark: self.theme.isDark,
            textColor: self.theme.colors.text,
            controlSize: self.size
        )
        .frame(width: self.width)
        .fixedSize(horizontal: false, vertical: true)
    }
}

private struct ThemedPopUpButton<Option: Hashable>: NSViewRepresentable {
    @Binding var selection: Option
    let options: [Option]
    let label: (Option) -> String
    let isDark: Bool
    let textColor: Color
    let controlSize: BobeControlSize

    func makeNSView(context: Context) -> NSPopUpButton {
        let button = NSPopUpButton(frame: .zero, pullsDown: false)
        button.target = context.coordinator
        button.action = #selector(Coordinator.changed(_:))
        self.configure(button)
        return button
    }

    func updateNSView(_ button: NSPopUpButton, context: Context) {
        context.coordinator.parent = self
        self.configure(button)
    }

    private func configure(_ button: NSPopUpButton) {
        button.appearance = NSAppearance(named: self.isDark ? .darkAqua : .aqua)
        button.isEnabled = !self.options.isEmpty
        button.contentTintColor = NSColor(self.textColor)
        switch self.controlSize {
        case .mini:
            button.controlSize = .mini
            button.font = .systemFont(ofSize: 10, weight: .medium)
        case .small:
            button.controlSize = .small
            button.font = .systemFont(ofSize: 11, weight: .medium)
        case .regular:
            button.controlSize = .regular
            button.font = .systemFont(ofSize: 13, weight: .medium)
        }

        let titles = self.options.map { self.label($0) }
        let current = (0 ..< button.numberOfItems).compactMap { button.item(at: $0)?.title }
        if current != titles {
            button.removeAllItems()
            button.addItems(withTitles: titles)
        }
        if let idx = self.options.firstIndex(of: self.selection) {
            button.selectItem(at: idx)
        } else if let fallback = self.options.first {
            self.selection = fallback
            button.selectItem(at: 0)
        }
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(self)
    }

    final class Coordinator: NSObject {
        var parent: ThemedPopUpButton
        init(_ parent: ThemedPopUpButton) {
            self.parent = parent
        }

        @MainActor @objc
        func changed(_ sender: NSPopUpButton) {
            let idx = sender.indexOfSelectedItem
            guard idx >= 0, idx < self.parent.options.count else { return }
            self.parent.selection = self.parent.options[idx]
        }
    }
}
