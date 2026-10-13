import SwiftUI

struct SoulsEditor: View {
    @State private var client = DaemonClient(endpoint: DaemonConfig.endpoint)

    var body: some View {
        EntityListEditor<Soul, SoulCreateRequest, SoulUpdateRequest>(
            strings: EntityEditorStrings(
                paneTitle: L10n.tr("settings.souls.title"),
                newPlaceholder: L10n.tr("settings.souls.new.placeholder"),
                loading: L10n.tr("settings.souls.loading"),
                emptyTitle: L10n.tr("settings.souls.empty.title"),
                emptyDescription: L10n.tr("settings.souls.empty.description"),
                emptySelectPrompt: L10n.tr("settings.souls.empty.select"),
                deleteAccessibility: L10n.tr("settings.souls.delete.accessibility"),
                toggleAccessibility: L10n.tr("settings.souls.toggle.enable_accessibility"),
                defaultContent: { name in "# \(name)\n\nDescribe this soul here.\n" }
            ),
            emptyIcon: "sparkles",
            actions: EntityActions(
                list: { [client = self.client] in try await client.listSouls().souls },
                create: { [client = self.client] request in try await client.createSoul(request) },
                update: { [client = self.client] id, request in try await client.updateSoul(id, request) },
                delete: { [client = self.client] id in try await client.deleteSoul(id) },
                enable: { [client = self.client] id in try await client.enableSoul(id) },
                disable: { [client = self.client] id in try await client.disableSoul(id) },
                buildCreateRequest: { name in
                    SoulCreateRequest(name: name, content: "# \(name)\n\nDescribe this soul here.\n")
                },
                buildUpdateRequest: { content in SoulUpdateRequest(content: content) },
                validate: { Validations.validateSoulContent($0) }
            )
        )
    }
}
