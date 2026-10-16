import SwiftUI

struct MemoryView: View {
    @Bindable var model: PhoneModel
    @State private var query = ""
    @Environment(\.theme) private var theme

    private var sections: [MemorySection] {
        MemorySection.parse(self.model.work.memory ?? "").filter { !$0.lines.isEmpty }.compactMap { $0.matching(self.query) }
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if self.model.needsAttention {
                        ConnectionCard(model: self.model)
                    } else if self.model.work.memoryLoad.failed, self.model.work.memory != nil {
                        PhoneTroubleNote(text: L10n.tr("phone.load.stale"))
                    }
                    if self.model.work.memory == nil {
                        if self.model.work.memoryLoad.failed {
                            PhoneTroubleNote(text: L10n.tr("phone.memory.failed"))
                        } else if self.model.isConnected {
                            ProgressView()
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 40)
                        }
                    } else if self.sections.isEmpty {
                        Text(L10n.tr(self.query.isEmpty ? "phone.memory.empty" : "phone.memory.no_match"))
                            .phoneText(.callout)
                            .foregroundStyle(self.theme.colors.textMuted)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(16)
                            .bobeWell(cornerRadius: BobeRadius.card)
                    }
                    ForEach(self.sections) { section in
                        PhoneCard {
                            VStack(alignment: .leading, spacing: 10) {
                                if let title = section.title {
                                    Text(title)
                                        .phoneText(.heading)
                                        .foregroundStyle(self.theme.colors.text)
                                }
                                ForEach(section.lines) { line in
                                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                                        if line.isBullet {
                                            Circle()
                                                .fill(self.theme.colors.primary.opacity(0.7))
                                                .frame(width: 5, height: 5)
                                                .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 4 }
                                        }
                                        Text(line.text)
                                            .phoneText(.body)
                                            .foregroundStyle(self.theme.colors.text)
                                            .fixedSize(horizontal: false, vertical: true)
                                            .textSelection(.enabled)
                                    }
                                }
                            }
                        }
                    }
                    if self.model.work.memory != nil {
                        Button(L10n.tr("phone.memory.forget")) { self.model.askToForget() }
                            .buttonStyle(.phoneSecondary)
                            .padding(.top, 4)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 8)
                .padding(.bottom, 24)
            }
            .background(PhoneBackdrop())
            .navigationTitle(L10n.tr("phone.memory.title"))
            .searchable(text: self.$query, prompt: L10n.tr("phone.memory.search"))
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    SettingsButton(model: self.model)
                }
            }
            .refreshable { await self.model.work.loadMemory() }
            .task(id: self.model.isConnected) {
                if self.model.isConnected {
                    await self.model.work.loadMemory()
                }
            }
        }
    }
}

struct MemorySection: Identifiable, Equatable {
    struct Line: Identifiable, Equatable {
        let id: Int
        let text: String
        let isBullet: Bool
    }

    let id: Int
    let title: String?
    let lines: [Line]

    static func parse(_ document: String) -> [MemorySection] {
        var sections: [MemorySection] = []
        var title: String?
        var lines: [Line] = []
        var counter = 0
        func flush() {
            if title != nil || !lines.isEmpty {
                sections.append(MemorySection(id: counter, title: title, lines: lines))
                counter += 1
            }
            lines = []
        }
        for raw in document.split(whereSeparator: \.isNewline) {
            let line = raw.trimmingCharacters(in: .whitespaces)
            guard !line.isEmpty else { continue }
            if line.hasPrefix("#") {
                flush()
                title = line.drop { $0 == "#" }.trimmingCharacters(in: .whitespaces)
                continue
            }
            let bullet = line.hasPrefix("- ") || line.hasPrefix("* ")
            let text = bullet ? String(line.dropFirst(2)) : line
            lines.append(Line(id: counter * 10000 + lines.count, text: text, isBullet: bullet))
        }
        flush()
        return sections
    }

    func matching(_ query: String) -> MemorySection? {
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !needle.isEmpty else { return self }
        if self.title?.localizedCaseInsensitiveContains(needle) == true { return self }
        let lines = self.lines.filter { $0.text.localizedCaseInsensitiveContains(needle) }
        return lines.isEmpty ? nil : MemorySection(id: self.id, title: self.title, lines: lines)
    }
}
