@testable import BoBe
import Foundation

actor SettingsClientStub: SettingsClient {
    enum ThemeOutcome {
        case failure
        case success(delivered: Bool)
    }

    private var settings: DaemonSettings
    private var themeOutcomes: [ThemeOutcome]
    private let restartRequiredFields: [String]
    private var failingFieldUpdates: Int
    private var themes: [PhysicalBodyTheme] = []
    private var requests: [SettingsUpdateRequest] = []
    private var shouldSuspendNextFetch = false
    private var fetchContinuation: CheckedContinuation<DaemonSettings, Never>?
    private var suspendUpdate = false
    private var updateContinuation: CheckedContinuation<Void, Never>?

    init(
        settings: DaemonSettings,
        themeOutcomes: [ThemeOutcome] = [],
        restartRequiredFields: [String] = [],
        failingFieldUpdates: Int = 0
    ) {
        self.settings = settings
        self.themeOutcomes = themeOutcomes
        self.restartRequiredFields = restartRequiredFields
        self.failingFieldUpdates = failingFieldUpdates
    }

    func getSettings() async -> DaemonSettings {
        if self.shouldSuspendNextFetch {
            self.shouldSuspendNextFetch = false
            return await withCheckedContinuation { continuation in
                self.fetchContinuation = continuation
            }
        }
        return self.settings
    }

    func updateSettings(_ request: SettingsUpdateRequest) async throws -> SettingsUpdateResponse {
        if self.suspendUpdate {
            self.suspendUpdate = false
            await withCheckedContinuation { self.updateContinuation = $0 }
        }
        self.requests.append(request)
        if let theme = request.bodyTheme {
            self.themes.append(theme)
            switch self.themeOutcomes.removeFirst() {
            case .failure:
                throw SettingsClientStubError.updateFailed
            case let .success(delivered):
                self.settings.bodyTheme = theme
                self.settings.bodyConnected = delivered
                return SettingsUpdateResponse(
                    message: "updated",
                    restartRequiredFields: self.restartRequiredFields,
                    persistFailed: nil,
                    bodyThemeDelivered: delivered
                )
            }
        }
        if self.failingFieldUpdates > 0 {
            self.failingFieldUpdates -= 1
            throw SettingsClientStubError.updateFailed
        }
        self.settings.apply(request)
        return SettingsUpdateResponse(
            message: "updated",
            restartRequiredFields: [],
            persistFailed: nil,
            bodyThemeDelivered: nil
        )
    }

    func suspendNextUpdate() { self.suspendUpdate = true }

    func hasSuspendedUpdate() -> Bool { self.updateContinuation != nil }

    func resumeUpdate() {
        self.updateContinuation?.resume()
        self.updateContinuation = nil
    }

    func suspendNextSettingsFetch() {
        self.shouldSuspendNextFetch = true
    }

    func hasSuspendedFetch() -> Bool {
        self.fetchContinuation != nil
    }

    func resumeSettingsFetch(with settings: DaemonSettings) {
        self.fetchContinuation?.resume(returning: settings)
        self.fetchContinuation = nil
    }

    func replaceSettings(_ settings: DaemonSettings) {
        self.settings = settings
    }

    func currentSettings() -> DaemonSettings {
        self.settings
    }

    func requestedThemes() -> [PhysicalBodyTheme] {
        self.themes
    }

    func sentRequests() -> [SettingsUpdateRequest] {
        self.requests
    }

    static func selection(generation: UInt64) -> EndpointSelection {
        EndpointSelection(generation: generation, profileID: UUID(), authorityID: UUID(), endpoint: .unavailable)
    }

    static func document(theme: PhysicalBodyTheme = .warmLedge) -> DaemonSettings {
        DaemonSettings(
            bodyTheme: theme,
            bodyConnected: true,
            captureEnabled: false,
            captureIntervalSeconds: 30,
            checkinEnabled: false,
            checkinTimes: [],
            checkinJitterMinutes: 0,
            conversationInactivityTimeoutSeconds: 300,
            conversationAutoCloseMinutes: 30,
            goalCheckIntervalSeconds: 60,
            mcpEnabled: false,
            voiceEnabled: true,
            voicePersona: "default",
            voiceSpeed: 1,
            voiceSttLanguage: "en",
            voicePauseSensitivity: "balanced",
            voiceShowPartialCaption: true
        )
    }
}

enum SettingsClientStubError: Error {
    case updateFailed
}
