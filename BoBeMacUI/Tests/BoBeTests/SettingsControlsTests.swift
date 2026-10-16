import AppKit
@testable import BoBe
import Foundation
import SwiftUI
import Testing

@MainActor
private final class Counter {
    var count = 0
}

@MainActor
private final class Commits {
    var values: [Int] = []
}

@Suite("Settings controls")
struct SettingsControlsTests {
    @Test
    @MainActor
    func failedRuntimeRestartIsShownAndCanBeRetried() async {
        let attempts = Counter()
        let request = RuntimeRestartRequest {
            attempts.count += 1
            if attempts.count == 1 {
                throw BackendServiceError.healthCheckFailed
            }
        }

        let first = await request.perform()

        #expect(!first)
        #expect(!request.isRestarting)
        #expect(request.errorMessage == L10n.tr("settings.shared.restart_banner.failed"))

        let retry = await request.perform()

        #expect(retry)
        #expect(request.errorMessage == nil)
        #expect(attempts.count == 2)
    }

    @Test
    @MainActor
    func rapidNumberEditsCommitOnlyTheSettledValue() async throws {
        let debouncer = NumberInputDebouncer(delay: .milliseconds(40))
        let commits = Commits()
        for text in ["1", "12", "120"] {
            debouncer.schedule(text, in: 1 ... 600) { commits.values.append($0) }
            await Task.yield()
        }
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        while commits.values.isEmpty, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
        try await Task.sleep(for: .milliseconds(120))

        #expect(commits.values == [120])

        debouncer.schedule("300", in: 1 ... 600) { commits.values.append($0) }
        debouncer.cancel()
        debouncer.schedule("900", in: 1 ... 600) { commits.values.append($0) }
        try await Task.sleep(for: .milliseconds(200))

        #expect(commits.values == [120])

        debouncer.commitNow("45", in: 1 ... 600) { commits.values.append($0) }

        #expect(commits.values == [120, 45])
    }

    @Test
    func expansionStateIsSpokenInTheInterfaceLanguage() {
        #expect(BobeExpansion.accessibilityValue(isExpanded: true) == L10n.tr("app.common.expanded"))
        #expect(BobeExpansion.accessibilityValue(isExpanded: false) == L10n.tr("app.common.collapsed"))
        #expect(BobeExpansion.accessibilityValue(isExpanded: true) != "app.common.expanded")
    }

    @Test
    @MainActor
    func rawEditorsNameTheirTextForAssistiveTechnology() throws {
        let editor = CodeEditor(
            text: .constant("# Soul"),
            accessibilityLabel: "Markdown for default",
            theme: allThemes[0]
        )
        let host = NSHostingView(rootView: editor.frame(width: 320, height: 200))
        let window = NSWindow(
            contentRect: CGRect(x: 0, y: 0, width: 320, height: 200),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        window.contentView = host
        host.layoutSubtreeIfNeeded()
        defer { window.close() }

        let textView = try #require(Self.firstSubview(of: NSTextView.self, in: host))

        #expect(textView.accessibilityLabel() == "Markdown for default")
    }

    @Test
    @MainActor
    func toolServerSavesLeaveSecretClassificationToTheDaemon() throws {
        let json = #"{"mcpServers":{"github":{"command":"npx","env":{"GITHUB_TOKEN":"ghp_example","API_KEY":"k"}}}}"#
        let request = MCPServersPanel.mutationRequest(for: json)
        let encoded = try #require(
            try JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any]
        )

        #expect(encoded["raw_json"] as? String == json)
        #expect(encoded["secret_keys"] == nil)
    }

    @MainActor
    private static func firstSubview<Found: NSView>(of type: Found.Type, in view: NSView) -> Found? {
        if let match = view as? Found {
            return match
        }
        for subview in view.subviews {
            if let match = self.firstSubview(of: type, in: subview) {
                return match
            }
        }
        return nil
    }
}
