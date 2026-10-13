import AppKit
import SwiftUI

extension Notification.Name {
    static let bobeWelcomeCompleted = Notification.Name("bobe.welcomeCompleted")
}

@MainActor
final class SetupWindowManager: NSObject, NSWindowDelegate {
    static let shared = SetupWindowManager()

    static let onboardingCompletedKey = "bobe.onboarding_completed"
    static let completedAuthoritiesKey = "bobe.surface_setup_completed_authorities"
    private static let openChatAfterOnboardingKey = "bobe.open_chat_after_onboarding"

    private var window: NSWindow?
    private var onCancel: (() -> Void)?
    private var onComplete: (() -> Void)?
    private var criticalOperation = false

    override private init() {}

    var isPresented: Bool {
        self.window != nil
    }

    static func isOnboardingCompleted(
        defaults: UserDefaults,
        authority: UUID?
    ) -> Bool {
        guard let authority else {
            return defaults.bool(forKey: Self.onboardingCompletedKey)
        }
        let completed = Set(defaults.stringArray(forKey: Self.completedAuthoritiesKey) ?? [])
        return completed.contains(authority.uuidString)
    }

    var isOnboardingCompleted: Bool {
        Self.isOnboardingCompleted(
            defaults: .standard,
            authority: EndpointStore.shared.runtimeAuthorityId
        )
    }

    func markOnboardingCompleted() {
        let defaults = UserDefaults.standard
        defaults.set(true, forKey: Self.onboardingCompletedKey)
        if let authority = EndpointStore.shared.runtimeAuthorityId {
            var completed = Set(defaults.stringArray(forKey: Self.completedAuthoritiesKey) ?? [])
            completed.insert(authority.uuidString)
            defaults.set(Array(completed).sorted(), forKey: Self.completedAuthoritiesKey)
        }
        defaults.set(true, forKey: Self.openChatAfterOnboardingKey)
    }

    func consumeOpenChatAfterOnboarding() -> Bool {
        let defaults = UserDefaults.standard
        guard defaults.bool(forKey: Self.openChatAfterOnboardingKey) else { return false }
        defaults.removeObject(forKey: Self.openChatAfterOnboardingKey)
        return true
    }

    @discardableResult
    func show(
        initialStep: WelcomeStep? = nil,
        runtimeRepairIssue: RuntimeIssue? = nil,
        completesAfterRuntimeRepair: Bool = false,
        marksOnboardingComplete: Bool = true,
        onCancel: (() -> Void)? = nil,
        onComplete: (() -> Void)? = nil
    ) -> Bool {
        if let window {
            window.makeKeyAndOrderFront(nil)
            NSApp.activate()
            return false
        }
        self.onCancel = onCancel
        self.onComplete = onComplete

        let view = WelcomeWizard(
            onComplete: { [weak self] in
                self?.onComplete?()
                self?.onComplete = nil
                guard marksOnboardingComplete else { return }
                self?.markOnboardingCompleted()
                NotificationCenter.default.post(name: .bobeWelcomeCompleted, object: nil)
            },
            initialStep: initialStep,
            runtimeRepairIssue: runtimeRepairIssue,
            completesAfterRuntimeRepair: completesAfterRuntimeRepair
        )

        let window = BobeWindowFactory.make(
            contentRect: NSRect(origin: .zero, size: WelcomeWizardMetrics.size),
            styleMask: [.titled, .closable, .fullSizeContentView],
            title: L10n.tr("setup.window.title"),
            animationBehavior: .documentWindow,
            rootView: view
        )
        window.setAccessibilityIdentifier("window.setup")
        window.delegate = self
        window.makeKeyAndOrderFront(nil)
        NSApp.activate()
        self.window = window
        return true
    }

    @discardableResult
    func showRuntimeRepair(issue: RuntimeIssue) -> Bool {
        self.show(
            initialStep: .runtimeLocation,
            runtimeRepairIssue: issue,
            completesAfterRuntimeRepair: true,
            marksOnboardingComplete: false,
            onComplete: {
                SetupWindowManager.shared.close()
                Task { @MainActor in
                    BobeStore.shared.repairIssue = nil
                    BobeStore.shared.connect()
                    OverlayWindowManager.shared.show()
                }
            }
        )
    }

    func setCriticalOperation(_ active: Bool) {
        self.criticalOperation = active
        self.window?.standardWindowButton(.closeButton)?.isEnabled = !active
    }

    func close() {
        self.window?.orderOut(nil)
        self.window?.contentViewController = nil
        self.window?.delegate = nil
        self.window = nil
        self.onCancel = nil
        self.onComplete = nil
        self.criticalOperation = false
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard !self.criticalOperation else {
            NSSound.beep()
            return false
        }
        if let onCancel {
            self.onCancel = nil
            onCancel()
            return true
        }
        guard !self.isOnboardingCompleted else { return true }

        let alert = NSAlert()
        alert.messageText = L10n.tr("app.setup_incomplete.title")
        alert.informativeText = L10n.tr("app.setup_incomplete.message")
        alert.alertStyle = .warning
        alert.addButton(withTitle: L10n.tr("app.setup_incomplete.retry"))
        alert.addButton(withTitle: L10n.tr("app.common.quit"))
        if alert.runModal() == .alertSecondButtonReturn {
            BobeCommandRouter.shared.quit()
        } else {
            sender.makeKeyAndOrderFront(nil)
            NSApp.activate()
        }
        return false
    }

    func windowWillClose(_ notification: Notification) {
        guard let closing = notification.object as? NSWindow, closing == self.window else { return }
        self.window = nil
        self.criticalOperation = false
    }
}
