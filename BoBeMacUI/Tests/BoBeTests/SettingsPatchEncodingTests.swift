@testable import BoBe
import Foundation
import Testing

@Suite("Settings PATCH encoding")
struct SettingsPatchEncodingTests {
    @Test
    func mcpEnabledEncodesAsDaemonField() throws {
        var request = SettingsUpdateRequest()
        request.mcpEnabled = false

        let encoded = try #require(
            JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any]
        )

        #expect(encoded["mcp_enabled"] as? Bool == false)
    }

    @Test
    func physicalBodyThemeUsesExactDaemonId() throws {
        var request = SettingsUpdateRequest()
        request.bodyTheme = .solarWideAwake

        let encoded = try #require(
            JSONSerialization.jsonObject(with: JSONEncoder().encode(request)) as? [String: Any]
        )

        #expect(encoded["body_theme"] as? String == "solar-wide-awake")
        #expect(PhysicalBodyTheme.allCases.map(\.rawValue) == [
            "warm-aperture",
            "warm-lintel",
            "warm-ledge",
            "kettle-tall-bright",
            "kettle-soft-bowl",
            "solar-wide-awake",
            "solar-crown",
        ])
    }
}
