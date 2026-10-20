import Foundation
import Observation

@Observable
final class RemoteSpeechConsent {
    static let shared = RemoteSpeechConsent()
    private static let keyPrefix = "bobe.voice.remote_stt_consent."

    private(set) var isEnabled = false
    private var identityObserver: NotificationCenter.ObservationToken?

    private init() {
        self.refresh()
        self.identityObserver = NotificationCenter.default.addObserver(
            for: EndpointIdentityChangedMessage.self
        ) { [weak self] _ in
            self?.refresh()
        }
    }

    var currentAuthorityEnabled: Bool {
        Self.currentKey.map(UserDefaults.standard.bool(forKey:)) ?? false
    }

    func refresh() {
        self.isEnabled = self.currentAuthorityEnabled
    }

    func setEnabled(_ enabled: Bool) {
        guard let key = Self.currentKey else {
            self.isEnabled = false
            return
        }
        self.isEnabled = enabled
        UserDefaults.standard.set(enabled, forKey: key)
    }

    static func storageKey(authorityId: UUID) -> String {
        Self.keyPrefix + authorityId.uuidString.lowercased()
    }

    private static var currentKey: String? {
        guard let authority = EndpointStore.shared.runtimeAuthorityId else { return nil }
        return Self.storageKey(authorityId: authority)
    }
}
