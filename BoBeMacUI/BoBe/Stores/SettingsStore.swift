import Foundation
import Observation
import OSLog
import SwiftUI

private let logger = Logger(subsystem: "com.bobe.app", category: "SettingsStore")

enum BodyThemeSaveState: Equatable {
    case idle
    case applying
    case applied
    case savedOffline
    case failed(String)

    var allowsRetry: Bool {
        switch self {
        case .savedOffline, .failed:
            true
        case .idle, .applying, .applied:
            false
        }
    }

    func refreshed(bodyConnected: Bool) -> Self {
        switch self {
        case .idle, .applying, .failed:
            return self
        case .applied, .savedOffline:
            return bodyConnected ? .applied : .savedOffline
        }
    }
}

protocol BodyThemeSettingsClient: Sendable {
    func getSettings() async throws -> DaemonSettings
    func updateSettings(_ request: SettingsUpdateRequest) async throws -> SettingsUpdateResponse
}

extension DaemonClient: BodyThemeSettingsClient {}

@MainActor
@Observable
final class SettingsStore {
    static let shared = SettingsStore()

    private(set) var settings: DaemonSettings?

    private(set) var error: String?

    private(set) var savedMessage: String?

    private(set) var bodyThemeSaveState = BodyThemeSaveState.idle

    private(set) var restartFields: Set<String> = []

    private(set) var isLoading = false

    private var loadTask: Task<Void, Never>?

    private var backendObserverTask: Task<Void, Never>?

    private let debouncer = SettingsDebouncer()
    private let clientOverride: (any BodyThemeSettingsClient)?
    private let currentSelection: @MainActor () -> EndpointSelection
    private var selection: EndpointSelection
    private var endpointObserver: NSObjectProtocol?
    private var bodyThemeGeneration: UInt64 = 0
    private var mutationRevision: UInt64 = 0

    init(
        bodyThemeClient: (any BodyThemeSettingsClient)? = nil,
        currentSelection: @escaping @MainActor () -> EndpointSelection = { EndpointStore.shared.selection }
    ) {
        self.clientOverride = bodyThemeClient
        self.currentSelection = currentSelection
        self.selection = currentSelection()
        self.endpointObserver = NotificationCenter.default.addObserver(
            forName: .bobeEndpointIdentityChanged, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                guard let self, self.selection != self.currentSelection() else { return }
                let wasLoaded = self.settings != nil || self.loadTask != nil
                self.synchronizeEndpoint()
                if wasLoaded { await self.loadIfNeeded() }
            }
        }
    }

    func synchronizeEndpoint() {
        let selected = self.currentSelection()
        guard selected != self.selection else { return }
        self.selection = selected
        self.debouncer.cancelPendingSave()
        self.loadTask?.cancel()
        self.loadTask = nil
        self.settings = nil
        self.error = nil
        self.savedMessage = nil
        self.restartFields = []
        self.bodyThemeSaveState = .idle
        self.isLoading = false
        self.bodyThemeGeneration &+= 1
        self.mutationRevision &+= 1
    }

    private func isCurrent(_ selection: EndpointSelection) -> Bool {
        self.selection == selection && self.currentSelection() == selection
    }

    private func client(for selection: EndpointSelection) -> any BodyThemeSettingsClient {
        self.clientOverride ?? DaemonClient(endpoint: selection.endpoint)
    }


    func loadIfNeeded(force: Bool = false) async {
        self.synchronizeEndpoint()
        self.startBackendObserverIfNeeded()
        if !force, self.settings != nil { return }
        if let existing = self.loadTask {
            await existing.value
            return
        }
        let selection = self.selection
        let revision = self.mutationRevision
        let bodyThemeGeneration = self.bodyThemeGeneration
        let client = self.client(for: selection)
        let task = Task<Void, Never> { @MainActor in
            self.isLoading = true
            defer { if self.isCurrent(selection) { self.isLoading = false } }
            do {
                let settings = try await client.getSettings()
                guard self.isCurrent(selection), revision == self.mutationRevision,
                      bodyThemeGeneration == self.bodyThemeGeneration,
                      self.bodyThemeSaveState != .applying
                else { return }
                self.settings = settings
                self.error = nil
                self.bodyThemeSaveState = .idle
            } catch {
                guard self.isCurrent(selection) else { return }
                self.error = error.localizedDescription
                logger.error("settings load failed: \(error.localizedDescription, privacy: .public)")
            }
        }
        self.loadTask = task
        await task.value
        if self.isCurrent(selection) { self.loadTask = nil }
    }

    private func startBackendObserverIfNeeded() {
        guard self.backendObserverTask == nil else { return }
        self.backendObserverTask = Task { @MainActor [weak self] in
            var wasCrashed = false
            let states = await BackendService.shared.stateStream
            for await state in states {
                guard !Task.isCancelled else { return }
                switch state {
                case .crashed, .fatal:
                    wasCrashed = true
                case .ready where wasCrashed:
                    wasCrashed = false
                    logger.info("daemon recovered — refreshing settings blob")
                    await self?.loadIfNeeded(force: true)
                case .ready, .starting, .stopped:
                    break
                }
            }
        }
    }

    func update(_ mutate: (inout DaemonSettings) -> Void) {
        self.synchronizeEndpoint()
        guard var current = self.settings else { return }
        mutate(&current)
        self.settings = current
        self.mutationRevision &+= 1
        let selection = self.selection
        let revision = self.mutationRevision
        let client = self.client(for: selection)
        self.debouncer.debounce { [weak self] in
            await self?.persist(
                currentSettings: current, selection: selection, revision: revision, client: client
            )
        }
    }

    func setBodyTheme(_ theme: PhysicalBodyTheme) async {
        self.synchronizeEndpoint()
        guard self.bodyThemeSaveState != .applying,
              var current = self.settings
        else {
            return
        }
        let retryingCurrentTheme =
            current.bodyTheme == theme && self.bodyThemeSaveState.allowsRetry
        guard current.bodyTheme != theme || retryingCurrentTheme else { return }
        current.bodyTheme = theme
        self.settings = current
        self.bodyThemeSaveState = .applying
        self.bodyThemeGeneration &+= 1
        let selection = self.selection
        let client = self.client(for: selection)

        var request = SettingsUpdateRequest()
        request.bodyTheme = theme
        do {
            let response = try await client.updateSettings(request)
            guard self.isCurrent(selection) else { return }
            if response.persistFailed == true {
                self.bodyThemeSaveState = .failed(
                    L10n.tr("settings.shared.action.persist_failed")
                )
                return
            }
            if let delivered = response.bodyThemeDelivered {
                self.settings?.bodyConnected = delivered
            }
            self.restartFields.formUnion(response.restartRequiredFields)
            self.bodyThemeSaveState = self.settings?.bodyConnected == true ? .applied : .savedOffline
        } catch {
            guard self.isCurrent(selection) else { return }
            self.bodyThemeSaveState = .failed(error.localizedDescription)
            logger.error(
                "physical body theme save failed: \(error.localizedDescription, privacy: .public)"
            )
        }
    }

    func refreshBodyThemeStatus() async {
        self.synchronizeEndpoint()
        guard self.bodyThemeSaveState != .applying else { return }
        let generation = self.bodyThemeGeneration
        let selection = self.selection
        let client = self.client(for: selection)
        do {
            let latest = try await client.getSettings()
            guard self.isCurrent(selection), generation == self.bodyThemeGeneration,
                  self.bodyThemeSaveState != .applying
            else {
                return
            }
            guard var current = self.settings else {
                self.settings = latest
                self.bodyThemeSaveState = self.bodyThemeSaveState.refreshed(
                    bodyConnected: latest.bodyConnected
                )
                return
            }
            if case .failed = self.bodyThemeSaveState {
                current.bodyConnected = latest.bodyConnected
                self.settings = current
                return
            }
            current.bodyTheme = latest.bodyTheme
            current.bodyConnected = latest.bodyConnected
            self.settings = current
            self.bodyThemeSaveState = self.bodyThemeSaveState.refreshed(
                bodyConnected: latest.bodyConnected
            )
        } catch {
            logger.debug(
                "physical body status refresh failed: \(error.localizedDescription, privacy: .public)"
            )
        }
    }

    private func persist(
        currentSettings: DaemonSettings,
        selection: EndpointSelection,
        revision: UInt64,
        client: any BodyThemeSettingsClient
    ) async {
        await self.debouncer.runPersist(
            setError: { [weak self] in self?.error = $0 },
            setSaved: { [weak self] in self?.savedMessage = $0 },
            onSuccess: { [weak self] response in
                guard let self else { return }
                self.restartFields.formUnion(response.restartRequiredFields)
                VoicePipeline.shared.applySettings(currentSettings)
            },
            update: { try await client.updateSettings($0) },
            isCurrent: { [weak self] in
                guard let self else { return false }
                return self.isCurrent(selection) && self.mutationRevision == revision
            },
            build: { req in
                req.captureEnabled = currentSettings.captureEnabled
                req.captureIntervalSeconds = currentSettings.captureIntervalSeconds
                req.checkinEnabled = currentSettings.checkinEnabled
                req.checkinTimes = currentSettings.checkinTimes
                req.checkinJitterMinutes = currentSettings.checkinJitterMinutes
                req.conversationInactivityTimeoutSeconds = currentSettings.conversationInactivityTimeoutSeconds
                req.conversationAutoCloseMinutes = currentSettings.conversationAutoCloseMinutes
                req.goalCheckIntervalSeconds = currentSettings.goalCheckIntervalSeconds
                req.mcpEnabled = currentSettings.mcpEnabled
                req.voiceEnabled = currentSettings.voiceEnabled
                req.voicePersona = currentSettings.voicePersona
                req.voiceSpeed = currentSettings.voiceSpeed
                req.voiceSttLanguage = currentSettings.voiceSttLanguage
                req.voicePauseSensitivity = currentSettings.voicePauseSensitivity
                req.voiceShowPartialCaption = currentSettings.voiceShowPartialCaption
            }
        )
    }

    func clearRestartFields() {
        self.restartFields.removeAll()
    }


    func binding<V>(
        _ keyPath: WritableKeyPath<DaemonSettings, V>,
        fallback: @autoclosure @escaping () -> V
    ) -> Binding<V> {
        Binding(
            get: { self.settings?[keyPath: keyPath] ?? fallback() },
            set: { [self] newValue in
                self.update { $0[keyPath: keyPath] = newValue }
            }
        )
    }

    func durationMinutesBinding(
        _ keyPath: WritableKeyPath<DaemonSettings, Double>,
        fallbackSeconds: @autoclosure @escaping () -> Int
    ) -> Binding<Int> {
        Binding(
            get: {
                let seconds = self.settings?[keyPath: keyPath] ?? Double(fallbackSeconds())
                return max(1, Int((seconds / 60).rounded()))
            },
            set: { [self] minutes in
                self.update {
                    $0[keyPath: keyPath] = Double(minutes * 60)
                }
            }
        )
    }


    func flushPendingSave() async {
        await self.debouncer.flush()
    }

    func cancelToast() {
        self.debouncer.cancelToast()
    }
}
