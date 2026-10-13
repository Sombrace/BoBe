import SwiftUI

struct HomeTabs: View {
    @Bindable var model: PhoneModel

    var body: some View {
        TabView(selection: self.$model.tab) {
            Tab(L10n.tr("phone.tab.bobe"), systemImage: "face.smiling", value: PhoneModel.Tab.bobe) {
                HomeView(model: self.model)
            }
            Tab(L10n.tr("phone.tab.work"), systemImage: "checklist", value: PhoneModel.Tab.work) {
                WorkView(model: self.model)
            }
            .badge(self.model.questions.count)
            Tab(L10n.tr("phone.tab.memory"), systemImage: "book.closed", value: PhoneModel.Tab.memory) {
                MemoryView(model: self.model)
            }
        }
        .sheet(isPresented: self.$model.showsSettings) {
            SettingsView(model: self.model)
        }
    }
}

struct PresenceTitle: View {
    let model: PhoneModel
    @Environment(\.theme) private var theme

    var body: some View {
        HStack(spacing: 8) {
            BobeCharacter(mood: self.model.mood, size: 26, palette: self.theme.avatar)
                .frame(width: 32, height: 32)
            VStack(alignment: .leading, spacing: 0) {
                Text(verbatim: "BoBe")
                    .phoneText(.heading)
                    .foregroundStyle(self.theme.colors.text)
                Text(self.model.status)
                    .phoneText(.footnote)
                    .foregroundStyle(self.theme.colors.textMuted)
                    .contentTransition(.opacity)
                    .lineLimit(1)
            }
        }
        .animation(BobeMotion.fade, value: self.model.status)
        .accessibilityElement(children: .combine)
    }
}

struct SettingsButton: View {
    let model: PhoneModel

    var body: some View {
        Button {
            self.model.showsSettings = true
        } label: {
            Image(systemName: "gearshape")
        }
        .accessibilityLabel(L10n.tr("phone.settings.title"))
        .accessibilityIdentifier("phone.settings.open")
    }
}
