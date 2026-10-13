import SwiftUI

struct UserProfilesEditor: View {
    @State private var client = DaemonClient(endpoint: DaemonConfig.endpoint)

    var body: some View {
        EntityListEditor<UserProfile, UserProfileCreateRequest, UserProfileUpdateRequest>(
            strings: EntityEditorStrings(
                paneTitle: L10n.tr("settings.user_profiles.title"),
                newPlaceholder: L10n.tr("settings.user_profiles.new.placeholder"),
                loading: L10n.tr("settings.user_profiles.loading"),
                emptyTitle: L10n.tr("settings.user_profiles.empty.title"),
                emptyDescription: L10n.tr("settings.user_profiles.empty.description"),
                emptySelectPrompt: L10n.tr("settings.user_profiles.empty.select"),
                deleteAccessibility: L10n.tr("settings.user_profiles.delete.accessibility"),
                toggleAccessibility: L10n.tr("settings.user_profiles.toggle.enable_accessibility"),
                defaultContent: { name in "# \(name)\n\nDescribe this profile here.\n" }
            ),
            emptyIcon: "person.crop.circle",
            actions: EntityActions(
                list: { [client = self.client] in try await client.listUserProfiles().profiles },
                create: { [client = self.client] request in try await client.createUserProfile(request) },
                update: { [client = self.client] id, request in try await client.updateUserProfile(id, request) },
                delete: { [client = self.client] id in try await client.deleteUserProfile(id) },
                enable: { [client = self.client] id in try await client.enableUserProfile(id) },
                disable: { [client = self.client] id in try await client.disableUserProfile(id) },
                buildCreateRequest: { name in
                    UserProfileCreateRequest(name: name, content: "# \(name)\n\nDescribe this profile here.\n")
                },
                buildUpdateRequest: { content in UserProfileUpdateRequest(content: content) },
                validate: { Validations.validateUserProfileContent($0) }
            )
        )
    }
}
