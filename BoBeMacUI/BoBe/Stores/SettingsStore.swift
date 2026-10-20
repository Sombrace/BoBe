import Foundation
import Observation
import OSLog
import SwiftUI

nonisolated private let logger = Logger(subsystem: "com.bobe.app", category: "SettingsStore")

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

nonisolated protocol SettingsClient: Sendable {
    func getSettings() async throws -> DaemonSettings
    func updateSettings(_ request: SettingsUpdateRequest) async throws -> SettingsUpdateResponse
}

extension DaemonClient: SettingsClient {}

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
    private let clientOverride: (any SettingsClient)?
    private let currentSelection: @MainActor () -> EndpointSelection
    private var selection: EndpointSelection
    private var endpointObserver: NotificationCenter.ObservationToken?
    private var bodyThemeGeneration: UInt64 = 0
    private var daemonSettings: DaemonSettings?
    private var pendingChanges = SettingsUpdateRequest()
    private var inFlightChanges = SettingsUpdateRequest()
    private var completedSaves: UInt64 = 0

    init(
        client: (any SettingsClient)? = nil,
        currentSelection: @escaping @MainActor () -> EndpointSelection = { EndpointStore.shared.selection }
    ) {
        self.clientOverride = client
        self.currentSelection = currentSelection
        self.selection = currentSelection()
        self.endpointObserver = NotificationCenter.default.addObserver(
            for: EndpointIdentityChangedMessage.self
        ) { [weak self] _ in
            guard let self, self.selection != self.currentSelection() else { return }
            let wasLoaded = self.settings != nil || self.loadTask != nil
            self.synchronizeEndpoint()
            if wasLoaded {
                Task(name: "Reload settings for endpoint") { await self.loadIfNeeded() }
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
        self.daemonSettings = nil
        self.pendingChanges = SettingsUpdateRequest()
        self.inFlightChanges = SettingsUpdateRequest()
        self.settings = nil
        self.error = nil
        self.savedMessage = nil
        self.restartFields = []
        self.bodyThemeSaveState = .idle
        self.isLoading = false
        self.bodyThemeGeneration &+= 1
    }

    private func isCurrent(_ selection: EndpointSelection) -> Bool {
        self.selection == selection && self.currentSelection() == selection
    }

    private func client(for selection: EndpointSelection) -> any SettingsClient {
        self.clientOverride ?? DaemonClient(endpoint: selection.endpoint)
    }

    private func publish() {
        guard var visible = self.daemonSettings else {
            self.settings = nil
            return
        }
        visible.apply(self.inFlightChanges)
        visible.apply(self.pendingChanges)
        if visible != self.settings {
            self.settings = visible
        }
    }

    private func adopt(_ latest: DaemonSettings, keepingBodyTheme: Bool) {
        var adopted = latest
        if keepingBodyTheme, let current = self.daemonSettings {
            adopted.bodyTheme = current.bodyTheme
            adopted.bodyConnected = current.bodyConnected
        }
        self.daemonSettings = adopted
        self.publish()
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
        let client = self.client(for: selection)
        let task = Task<Void, Never> {
            self.isLoading = true
            defer { if self.isCurrent(selection) { self.isLoading = false } }
            await self.debouncer.flush()
            guard self.isCurrent(selection) else { return }
            let bodyThemeGeneration = self.bodyThemeGeneration
            let completedSaves = self.completedSaves
            do {
                let latest = try await client.getSettings()
                guard self.isCurrent(selection), completedSaves == self.completedSaves else { return }
                let bodyThemeSettled = bodyThemeGeneration == self.bodyThemeGeneration
                    && self.bodyThemeSaveState != .applying
                self.adopt(latest, keepingBodyTheme: !bodyThemeSettled)
                if bodyThemeSettled {
                    self.bodyThemeSaveState = .idle
                }
                self.error = nil
            } catch {
                guard self.isCurrent(selection) else { return }
                self.error = error.localizedDescription
                logger.error("settings load failed: \(error.localizedDescription, privacy: .public)")
            }
        }
        self.loadTask = task
        await task.value
        if self.loadTask == task { self.loadTask = nil }
    }

    func refresh() async {
        self.synchronizeEndpoint()
        guard self.settings != nil else {
            await self.loadIfNeeded()
            return
        }
        if let existing = self.loadTask {
            await existing.value
            return
        }
        let selection = self.selection
        let completedSaves = self.completedSaves
        guard let latest = try? await self.client(for: selection).getSettings(),
              self.isCurrent(selection),
              completedSaves == self.completedSaves,
              self.daemonSettings != nil
        else { return }
        self.adopt(latest, keepingBodyTheme: true)
    }

    private func startBackendObserverIfNeeded() {
        guard self.backendObserverTask == nil else { return }
        self.backendObserverTask = Task(name: "Observe daemon settings") { [weak self] in
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
        guard let previous = self.settings else { return }
        var edited = previous
        mutate(&edited)
        let changes = SettingsUpdateRequest(changing: previous, to: edited)
        guard !changes.isEmpty else { return }
        self.pendingChanges.merge(changes)
        self.publish()
        let selection = self.selection
        let client = self.client(for: selection)
        self.debouncer.debounce { [weak self] in
            await self?.persistPendingChanges(selection: selection, client: client)
        }
    }

    @discardableResult
    func setCaptureEnabled(_ enabled: Bool) async -> Bool {
        await self.loadIfNeeded()
        guard self.settings != nil else { return false }
        self.update { $0.captureEnabled = enabled }
        await self.flushPendingSave()
        return self.settings?.captureEnabled == enabled
    }

    func setBodyTheme(_ theme: PhysicalBodyTheme) async {
        self.synchronizeEndpoint()
        guard self.bodyThemeSaveState != .applying,
              var current = self.daemonSettings
        else {
            return
        }
        let retryingCurrentTheme =
            current.bodyTheme == theme && self.bodyThemeSaveState.allowsRetry
        guard current.bodyTheme != theme || retryingCurrentTheme else { return }
        current.bodyTheme = theme
        self.daemonSettings = current
        self.publish()
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
                self.daemonSettings?.bodyConnected = delivered
                self.publish()
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
            guard var current = self.daemonSettings else {
                self.daemonSettings = latest
                self.publish()
                self.bodyThemeSaveState = self.bodyThemeSaveState.refreshed(
                    bodyConnected: latest.bodyConnected
                )
                return
            }
            current.bodyConnected = latest.bodyConnected
            if case .failed = self.bodyThemeSaveState {
                self.daemonSettings = current
                self.publish()
                return
            }
            current.bodyTheme = latest.bodyTheme
            self.daemonSettings = current
            self.publish()
            self.bodyThemeSaveState = self.bodyThemeSaveState.refreshed(
                bodyConnected: latest.bodyConnected
            )
        } catch {
            logger.debug(
                "physical body status refresh failed: \(error.localizedDescription, privacy: .public)"
            )
        }
    }

    private func persistPendingChanges(
        selection: EndpointSelection,
        client: any SettingsClient
    ) async {
        guard self.isCurrent(selection), !self.pendingChanges.isEmpty else { return }
        let changes = self.pendingChanges
        self.pendingChanges = SettingsUpdateRequest()
        self.inFlightChanges = changes
        await self.debouncer.runPersist(
            setError: { [weak self] in self?.error = $0 },
            setSaved: { [weak self] in self?.savedMessage = $0 },
            onSuccess: { [weak self] response in
                self?.daemonSettings?.apply(changes)
                self?.restartFields.formUnion(response.restartRequiredFields)
            },
            update: { try await client.updateSettings($0) },
            isCurrent: { [weak self] in self?.isCurrent(selection) ?? false },
            build: { $0 = changes }
        )
        guard self.isCurrent(selection) else { return }
        self.inFlightChanges = SettingsUpdateRequest()
        self.completedSaves &+= 1
        self.publish()
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

extension SettingsUpdateRequest {
    init(changing previous: DaemonSettings, to current: DaemonSettings) {
        func changed<Value: Equatable>(_ field: KeyPath<DaemonSettings, Value>) -> Value? {
            previous[keyPath: field] == current[keyPath: field] ? nil : current[keyPath: field]
        }
        self.init(
            bodyTheme: changed(\.bodyTheme),
            captureEnabled: changed(\.captureEnabled),
            captureIntervalSeconds: changed(\.captureIntervalSeconds),
            checkinEnabled: changed(\.checkinEnabled),
            checkinTimes: changed(\.checkinTimes),
            checkinJitterMinutes: changed(\.checkinJitterMinutes),
            conversationInactivityTimeoutSeconds: changed(\.conversationInactivityTimeoutSeconds),
            conversationAutoCloseMinutes: changed(\.conversationAutoCloseMinutes),
            goalCheckIntervalSeconds: changed(\.goalCheckIntervalSeconds),
            mcpEnabled: changed(\.mcpEnabled),
            voiceEnabled: changed(\.voiceEnabled),
            voicePersona: changed(\.voicePersona),
            voiceSpeed: changed(\.voiceSpeed),
            voiceSttLanguage: changed(\.voiceSttLanguage),
            voicePauseSensitivity: changed(\.voicePauseSensitivity),
            voiceShowPartialCaption: changed(\.voiceShowPartialCaption)
        )
    }

    var isEmpty: Bool {
        self == SettingsUpdateRequest()
    }

    mutating func merge(_ newer: SettingsUpdateRequest) {
        self = SettingsUpdateRequest(
            bodyTheme: newer.bodyTheme ?? self.bodyTheme,
            captureEnabled: newer.captureEnabled ?? self.captureEnabled,
            captureIntervalSeconds: newer.captureIntervalSeconds ?? self.captureIntervalSeconds,
            checkinEnabled: newer.checkinEnabled ?? self.checkinEnabled,
            checkinTimes: newer.checkinTimes ?? self.checkinTimes,
            checkinJitterMinutes: newer.checkinJitterMinutes ?? self.checkinJitterMinutes,
            conversationInactivityTimeoutSeconds: newer.conversationInactivityTimeoutSeconds
                ?? self.conversationInactivityTimeoutSeconds,
            conversationAutoCloseMinutes: newer.conversationAutoCloseMinutes ?? self.conversationAutoCloseMinutes,
            goalCheckIntervalSeconds: newer.goalCheckIntervalSeconds ?? self.goalCheckIntervalSeconds,
            mcpEnabled: newer.mcpEnabled ?? self.mcpEnabled,
            voiceEnabled: newer.voiceEnabled ?? self.voiceEnabled,
            voicePersona: newer.voicePersona ?? self.voicePersona,
            voiceSpeed: newer.voiceSpeed ?? self.voiceSpeed,
            voiceSttLanguage: newer.voiceSttLanguage ?? self.voiceSttLanguage,
            voicePauseSensitivity: newer.voicePauseSensitivity ?? self.voicePauseSensitivity,
            voiceShowPartialCaption: newer.voiceShowPartialCaption ?? self.voiceShowPartialCaption
        )
    }
}

nonisolated extension DaemonSettings {
    mutating func apply(_ changes: SettingsUpdateRequest) {
        self.bodyTheme = changes.bodyTheme ?? self.bodyTheme
        self.captureEnabled = changes.captureEnabled ?? self.captureEnabled
        self.captureIntervalSeconds = changes.captureIntervalSeconds ?? self.captureIntervalSeconds
        self.checkinEnabled = changes.checkinEnabled ?? self.checkinEnabled
        self.checkinTimes = changes.checkinTimes ?? self.checkinTimes
        self.checkinJitterMinutes = changes.checkinJitterMinutes ?? self.checkinJitterMinutes
        self.conversationInactivityTimeoutSeconds = changes.conversationInactivityTimeoutSeconds
            ?? self.conversationInactivityTimeoutSeconds
        self.conversationAutoCloseMinutes = changes.conversationAutoCloseMinutes ?? self.conversationAutoCloseMinutes
        self.goalCheckIntervalSeconds = changes.goalCheckIntervalSeconds ?? self.goalCheckIntervalSeconds
        self.mcpEnabled = changes.mcpEnabled ?? self.mcpEnabled
        self.voiceEnabled = changes.voiceEnabled ?? self.voiceEnabled
        self.voicePersona = changes.voicePersona ?? self.voicePersona
        self.voiceSpeed = changes.voiceSpeed ?? self.voiceSpeed
        self.voiceSttLanguage = changes.voiceSttLanguage ?? self.voiceSttLanguage
        self.voicePauseSensitivity = changes.voicePauseSensitivity ?? self.voicePauseSensitivity
        self.voiceShowPartialCaption = changes.voiceShowPartialCaption ?? self.voiceShowPartialCaption
    }
}
