import AppKit
import Observation
import SwiftUI

@Observable
final class AccessibilityRuntime {
    static let shared = AccessibilityRuntime()

    private(set) var reduceMotion: Bool

    @ObservationIgnored private var displayOptionsObserver: NotificationCenter.ObservationToken?

    private init(workspace: NSWorkspace = .shared) {
        self.reduceMotion = workspace.accessibilityDisplayShouldReduceMotion
        self.displayOptionsObserver = workspace.notificationCenter.addObserver(
            of: workspace,
            for: .accessibilityDisplayOptionsDidChange
        ) { [weak self] _ in
            self?.refresh(from: .shared)
        }
    }

    private func refresh(from workspace: NSWorkspace) {
        self.reduceMotion = workspace.accessibilityDisplayShouldReduceMotion
    }
}
