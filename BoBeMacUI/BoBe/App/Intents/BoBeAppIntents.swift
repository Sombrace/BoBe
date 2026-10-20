import AppIntents
import AppKit

nonisolated let intentLocalizationBundle = LocalizedStringResource.BundleDescription.atURL(
    Bundle.appResources.bundleURL
)

private enum BobeIntentError: LocalizedError {
    case setupRequired
    case unavailable

    var errorDescription: String? {
        switch self {
        case .setupRequired:
            L10n.tr("intents.error.setup_required")
        case .unavailable:
            L10n.tr("intents.error.unavailable")
        }
    }
}

private func requirePerformed(_ outcome: BobeCommandOutcome) throws {
    switch outcome {
    case .performed:
        return
    case .setupRequired:
        throw BobeIntentError.setupRequired
    case .unavailable:
        throw BobeIntentError.unavailable
    }
}

struct ShowBobeIntent: AppIntent {
    static let title = LocalizedStringResource("intents.show.title", table: "UI", bundle: intentLocalizationBundle)
    static let description = IntentDescription(
        LocalizedStringResource("intents.show.description", table: "UI", bundle: intentLocalizationBundle)
    )
    static let openAppWhenRun = true

    @MainActor
    func perform() async throws -> some IntentResult {
        NSApp.activate()
        try requirePerformed(BobeCommandRouter.shared.showOverlay())
        return .result()
    }
}

struct HideBobeIntent: AppIntent {
    static let title = LocalizedStringResource("intents.hide.title", table: "UI", bundle: intentLocalizationBundle)
    static let description = IntentDescription(
        LocalizedStringResource("intents.hide.description", table: "UI", bundle: intentLocalizationBundle)
    )
    static let openAppWhenRun = false

    @MainActor
    func perform() async throws -> some IntentResult {
        try requirePerformed(BobeCommandRouter.shared.hideOverlay())
        return .result()
    }
}

struct OpenBobeSettingsIntent: AppIntent {
    static let title = LocalizedStringResource("intents.settings.title", table: "UI", bundle: intentLocalizationBundle)
    static let description = IntentDescription(
        LocalizedStringResource("intents.settings.description", table: "UI", bundle: intentLocalizationBundle)
    )
    static let openAppWhenRun = false

    @MainActor
    func perform() async throws -> some IntentResult {
        try requirePerformed(BobeCommandRouter.shared.openSettings())
        return .result()
    }
}

struct ToggleBobeMicrophoneMuteIntent: AppIntent {
    static let title = LocalizedStringResource("intents.mute.title", table: "UI", bundle: intentLocalizationBundle)
    static let description = IntentDescription(
        LocalizedStringResource("intents.mute.description", table: "UI", bundle: intentLocalizationBundle)
    )
    static let openAppWhenRun = false

    @MainActor
    func perform() async throws -> some IntentResult {
        try requirePerformed(await BobeCommandRouter.shared.toggleLogicalMicrophoneMute())
        return .result()
    }
}

struct StopBobeSpeakingIntent: AppIntent {
    static let title = LocalizedStringResource("intents.stop_speaking.title", table: "UI", bundle: intentLocalizationBundle)
    static let description = IntentDescription(
        LocalizedStringResource("intents.stop_speaking.description", table: "UI", bundle: intentLocalizationBundle)
    )
    static let openAppWhenRun = false

    @MainActor
    func perform() async throws -> some IntentResult {
        try requirePerformed(BobeCommandRouter.shared.stopSpeaking())
        return .result()
    }
}
