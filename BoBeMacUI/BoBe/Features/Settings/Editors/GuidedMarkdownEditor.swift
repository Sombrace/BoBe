import SwiftUI

enum ContextEditorMode: String, CaseIterable {
    case guided
    case raw

    var label: String {
        L10n.tr("settings.context_editor.mode.\(self.rawValue)")
    }
}

struct MarkdownSectionDraft: Identifiable, Equatable {
    let id: UUID
    var title: String
    var body: String

    init(id: UUID = UUID(), title: String, body: String) {
        self.id = id
        self.title = title
        self.body = body
    }
}

struct MarkdownDocumentDraft: Equatable {
    var title: String
    var introduction: String
    var sections: [MarkdownSectionDraft]

    static func parse(_ markdown: String, fallbackTitle: String) -> MarkdownDocumentDraft {
        var title = fallbackTitle
        var introductionLines: [String] = []
        var sections: [MarkdownSectionDraft] = []
        var currentTitle: String?
        var currentLines: [String] = []
        var fenceMarker: String?

        func flushSection() {
            guard let currentTitle else { return }
            sections.append(
                MarkdownSectionDraft(
                    title: currentTitle,
                    body: Self.trimBlankLines(currentLines).joined(separator: "\n")
                )
            )
        }

        for line in markdown.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if let marker = fenceMarker {
                if trimmed.hasPrefix(marker) {
                    fenceMarker = nil
                }
                if currentTitle == nil {
                    introductionLines.append(line)
                } else {
                    currentLines.append(line)
                }
            } else if trimmed.hasPrefix("```") || trimmed.hasPrefix("~~~") {
                fenceMarker = String(trimmed.prefix(3))
                if currentTitle == nil {
                    introductionLines.append(line)
                } else {
                    currentLines.append(line)
                }
            } else if line.hasPrefix("# "), sections.isEmpty, currentTitle == nil, introductionLines.isEmpty {
                title = String(line.dropFirst(2)).trimmingCharacters(in: .whitespaces)
            } else if line.hasPrefix("## ") {
                flushSection()
                currentTitle = String(line.dropFirst(3)).trimmingCharacters(in: .whitespaces)
                currentLines = []
            } else if currentTitle == nil {
                introductionLines.append(line)
            } else {
                currentLines.append(line)
            }
        }
        flushSection()

        var introduction = Self.trimBlankLines(introductionLines).joined(separator: "\n")
        if sections.isEmpty {
            let promoted = Self.promoteColonSections(in: introduction)
            introduction = promoted.introduction
            sections = promoted.sections
        }

        return MarkdownDocumentDraft(
            title: title,
            introduction: introduction,
            sections: sections
        )
    }

    func render() -> String {
        var blocks = ["# \(self.title.trimmingCharacters(in: .whitespacesAndNewlines))"]
        let intro = self.introduction.trimmingCharacters(in: .whitespacesAndNewlines)
        if !intro.isEmpty {
            blocks.append(intro)
        }
        for section in self.sections {
            let title = section.title.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !title.isEmpty else { continue }
            let body = section.body.trimmingCharacters(in: .whitespacesAndNewlines)
            blocks.append(body.isEmpty ? "## \(title)" : "## \(title)\n\n\(body)")
        }
        return blocks.joined(separator: "\n\n") + "\n"
    }

    private static func trimBlankLines(_ lines: [String]) -> ArraySlice<String> {
        let first = lines.firstIndex { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        let last = lines.lastIndex { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        guard let first, let last else { return [] }
        return lines[first ... last]
    }

    private static func promoteColonSections(
        in introduction: String
    ) -> (introduction: String, sections: [MarkdownSectionDraft]) {
        var overview: [String] = []
        var sections: [MarkdownSectionDraft] = []
        var currentTitle: String?
        var currentLines: [String] = []

        func flush() {
            guard let currentTitle else { return }
            sections.append(
                MarkdownSectionDraft(
                    title: currentTitle,
                    body: Self.trimBlankLines(currentLines).joined(separator: "\n")
                )
            )
        }

        for line in introduction.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            let isSectionHeading = trimmed.hasSuffix(":")
                && trimmed.count <= 60
                && !trimmed.hasPrefix("-")
                && !trimmed.hasPrefix("*")
            if isSectionHeading {
                flush()
                currentTitle = String(trimmed.dropLast())
                currentLines = []
            } else if currentTitle == nil {
                overview.append(line)
            } else {
                currentLines.append(line)
            }
        }
        flush()

        return (
            Self.trimBlankLines(overview).joined(separator: "\n"),
            sections
        )
    }
}

struct GuidedMarkdownEditor: View {
    @Binding var text: String
    let fallbackTitle: String

    @State private var draft: MarkdownDocumentDraft
    @State private var lastRendered: String
    @State private var isSynchronizing = false
    @Environment(\.theme) private var theme

    init(text: Binding<String>, fallbackTitle: String) {
        self._text = text
        self.fallbackTitle = fallbackTitle
        let initial = MarkdownDocumentDraft.parse(text.wrappedValue, fallbackTitle: fallbackTitle)
        self._draft = State(initialValue: initial)
        self._lastRendered = State(initialValue: initial.render())
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                self.field(
                    label: L10n.tr("settings.context_editor.title"),
                    help: L10n.tr("settings.context_editor.title.help")
                ) {
                    TextField(
                        L10n.tr("settings.context_editor.title.placeholder"),
                        text: self.$draft.title
                    )
                    .bobeField()
                }

                self.field(
                    label: L10n.tr("settings.context_editor.overview"),
                    help: L10n.tr("settings.context_editor.overview.help")
                ) {
                    BobeProseEditor(text: self.$draft.introduction, minHeight: 110)
                }

                ForEach(self.$draft.sections) { $section in
                    VStack(alignment: .leading, spacing: 8) {
                        HStack(spacing: 8) {
                            TextField(
                                L10n.tr("settings.context_editor.section.placeholder"),
                                text: $section.title
                            )
                            .textFieldStyle(.plain)
                            .bobeTextStyle(.rowTitle)

                            Spacer()

                            Button(
                                action: { self.removeSection(section.id) },
                                label: { Image(systemName: "trash") }
                            )
                            .buttonStyle(.plain)
                            .foregroundStyle(self.theme.colors.primary)
                            .accessibilityLabel(self.deleteLabel(for: section.title))
                        }

                        BobeProseEditor(text: $section.body, minHeight: 96)
                    }
                    .padding(12)
                    .bobeRaised(cornerRadius: 16)
                }

                Button {
                    self.draft.sections.append(
                        MarkdownSectionDraft(
                            title: L10n.tr("settings.context_editor.section.new_title"),
                            body: ""
                        )
                    )
                } label: {
                    Label(
                        L10n.tr("settings.context_editor.section.add"),
                        systemImage: "plus"
                    )
                }
                .bobeButton(.secondary, size: .small)
            }
            .padding(2)
        }
        .onChange(of: self.draft) { _, newDraft in
            if self.isSynchronizing {
                self.isSynchronizing = false
                return
            }
            let rendered = newDraft.render()
            self.lastRendered = rendered
            if self.text != rendered {
                self.text = rendered
            }
        }
        .onChange(of: self.text) { _, newText in
            guard newText != self.lastRendered else { return }
            let parsed = MarkdownDocumentDraft.parse(newText, fallbackTitle: self.fallbackTitle)
            self.isSynchronizing = true
            self.draft = parsed
            self.lastRendered = parsed.render()
        }
    }

    private func removeSection(_ id: UUID) {
        self.draft.sections.removeAll { $0.id == id }
    }

    private func deleteLabel(for title: String) -> String {
        L10n.tr("settings.context_editor.section.delete_accessibility_format", title)
    }

    private func field(
        label: String,
        help: String,
        @ViewBuilder content: () -> some View
    ) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(label)
                .bobeTextStyle(.rowTitle)
                .foregroundStyle(self.theme.colors.text)
            content()
            Text(help)
                .bobeTextStyle(.helper)
                .foregroundStyle(self.theme.colors.textMuted)
        }
    }
}

struct BobeProseEditor: View {
    @Binding var text: String
    var minHeight: CGFloat

    @Environment(\.theme) private var theme

    var body: some View {
        TextEditor(text: self.$text)
            .font(.body)
            .foregroundStyle(self.theme.colors.text)
            .scrollContentBackground(.hidden)
            .padding(8)
            .frame(minHeight: self.minHeight)
            .background(
                RoundedRectangle(cornerRadius: 8)
                    .fill(self.theme.colors.background)
                    .stroke(self.theme.colors.border, lineWidth: 1)
            )
    }
}
