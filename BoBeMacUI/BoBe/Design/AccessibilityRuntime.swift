import AppKit
import Observation
import SwiftUI

@MainActor
@Observable
final class AccessibilityRuntime {
    static let shared = AccessibilityRuntime()

    private(set) var reduceMotion: Bool

    @ObservationIgnored private var displayOptionsObserver: NSObjectProtocol?

    private init(workspace: NSWorkspace = .shared) {
        self.reduceMotion = workspace.accessibilityDisplayShouldReduceMotion
        self.displayOptionsObserver = workspace.notificationCenter.addObserver(
            forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.refresh(from: .shared)
            }
        }
    }

    private func refresh(from workspace: NSWorkspace) {
        self.reduceMotion = workspace.accessibilityDisplayShouldReduceMotion
    }
}
