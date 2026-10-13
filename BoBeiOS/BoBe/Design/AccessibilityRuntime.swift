import Observation
import UIKit

@MainActor
@Observable
final class AccessibilityRuntime {
    static let shared = AccessibilityRuntime()

    private(set) var reduceMotion: Bool

    @ObservationIgnored private var observer: NSObjectProtocol?

    private init() {
        self.reduceMotion = UIAccessibility.isReduceMotionEnabled
        self.observer = NotificationCenter.default.addObserver(
            forName: UIAccessibility.reduceMotionStatusDidChangeNotification,
            object: nil,
            queue: .main
        ) { _ in
            Task { @MainActor in
                AccessibilityRuntime.shared.refresh()
            }
        }
    }

    private func refresh() {
        self.reduceMotion = UIAccessibility.isReduceMotionEnabled
    }
}
