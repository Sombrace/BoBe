import Observation
import UIKit

@Observable
final class AccessibilityRuntime {
    static let shared = AccessibilityRuntime()

    private(set) var reduceMotion: Bool

    @ObservationIgnored private var observer: NotificationCenter.ObservationToken?

    private init() {
        self.reduceMotion = UIAccessibility.isReduceMotionEnabled
        self.observer = NotificationCenter.default.addObserver(
            of: UIAccessibility.self,
            for: .reduceMotionStatusDidChange
        ) { _ in
            AccessibilityRuntime.shared.refresh()
        }
    }

    private func refresh() {
        self.reduceMotion = UIAccessibility.isReduceMotionEnabled
    }
}
