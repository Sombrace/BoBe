import Foundation
import XCTest

final class BoBeUITests: XCTestCase {
    override func setUp() {
        super.setUp()
        self.continueAfterFailure = false
    }

    @MainActor
    func testOnboardingStartsWithProductValue() {
        let app = self.makeApp()
        app.launch()
        defer { app.terminate() }

        let continueButton = app.buttons["setup.value.continue"]
        XCTAssertTrue(continueButton.waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["A companion that learns what matters"].exists)
    }

    @MainActor
    func testSettingsSearchFindsVoiceSpeed() {
        let app = self.makeApp()
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["setup.value.continue"].waitForExistence(timeout: 5))

        app.typeKey(",", modifierFlags: .command)

        let settingsWindow = app.windows["BoBe Settings"]
        XCTAssertTrue(settingsWindow.waitForExistence(timeout: 5))
        XCTAssertGreaterThanOrEqual(settingsWindow.frame.width, 1100)
        XCTAssertGreaterThanOrEqual(settingsWindow.frame.height, 720)

        let search = app.textFields["settings.search"]
        XCTAssertTrue(search.waitForExistence(timeout: 3))
        search.click()
        search.typeText("speed")

        XCTAssertTrue(app.buttons["settings.sidebar.voice"].waitForExistence(timeout: 3))
        XCTAssertFalse(app.staticTexts["No settings match \"speed\"."].exists)
    }

    @MainActor
    private func makeApp() -> XCUIApplication {
        let app = XCUIApplication()
        let dataDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("bobe-ui-tests-\(UUID().uuidString)", isDirectory: true)
        self.addTeardownBlock {
            try? FileManager.default.removeItem(at: dataDirectory)
        }
        let port = 20_000 + Int(ProcessInfo.processInfo.processIdentifier % 20_000)
        app.launchEnvironment["BOBE_DEV"] = "1"
        app.launchEnvironment["BOBE_MANAGED_DATA_DIR"] = dataDirectory.path
        app.launchEnvironment["BOBE_MANAGED_PORT"] = String(port)
        app.launchArguments += ["-bobe.onboarding_completed", "NO"]
        return app
    }
}
