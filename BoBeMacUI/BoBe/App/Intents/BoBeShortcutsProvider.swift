import AppIntents

struct BoBeShortcutsProvider: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(
            intent: ShowBobeIntent(),
            phrases: ["Show \(.applicationName)"],
            shortTitle: LocalizedStringResource("intents.show.title", table: "UI", bundle: intentLocalizationBundle),
            systemImageName: "bubble.left.and.bubble.right.fill"
        )
        AppShortcut(
            intent: HideBobeIntent(),
            phrases: ["Hide \(.applicationName)"],
            shortTitle: LocalizedStringResource("intents.hide.title", table: "UI", bundle: intentLocalizationBundle),
            systemImageName: "eye.slash.fill"
        )
        AppShortcut(
            intent: OpenBobeSettingsIntent(),
            phrases: ["Open \(.applicationName) settings"],
            shortTitle: LocalizedStringResource("intents.settings.title", table: "UI", bundle: intentLocalizationBundle),
            systemImageName: "gearshape.fill"
        )
        AppShortcut(
            intent: ToggleBobeMicrophoneMuteIntent(),
            phrases: ["Toggle \(.applicationName) microphone mute"],
            shortTitle: LocalizedStringResource("intents.mute.title", table: "UI", bundle: intentLocalizationBundle),
            systemImageName: "mic.slash.fill"
        )
        AppShortcut(
            intent: StopBobeSpeakingIntent(),
            phrases: ["Stop \(.applicationName) speaking"],
            shortTitle: LocalizedStringResource("intents.stop_speaking.title", table: "UI", bundle: intentLocalizationBundle),
            systemImageName: "stop.fill"
        )
    }
}
