import Foundation
import Observation

@Observable
final class ExpertMode {
    static let shared = ExpertMode()

    private static let key = "bobe.expert_mode"

    private(set) var isEnabled: Bool

    private init() {
        self.isEnabled = UserDefaults.standard.bool(forKey: Self.key)
    }

    func setEnabled(_ enabled: Bool) {
        guard self.isEnabled != enabled else { return }
        self.isEnabled = enabled
        UserDefaults.standard.set(enabled, forKey: Self.key)
    }
}
