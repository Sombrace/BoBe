@testable import BoBe
import AppKit
import Testing

@Suite("Provider brand assets")
struct ProviderBrandAssetTests {
    @Test
    @MainActor
    func providerLogosLoadFromTheAppBundleInBothAppearances() throws {
        for mode in [NSAppearance.Name.aqua, .darkAqua] {
            let appearance = try #require(NSAppearance(named: mode))
            for kind in ProviderKindWire.allCases where kind != .openAICompatible {
                let name = try #require(kind.logoAssetName)
                var loaded: NSImage?
                appearance.performAsCurrentDrawingAppearance {
                    loaded = Bundle.appResources.image(forResource: NSImage.Name(name))
                }
                let image = try #require(loaded, "Missing bundled logo for \(kind)")
                #expect(image.size.width > 0 && image.size.height > 0)
                #expect(image.isTemplate == kind.usesMonochromeLogo)
            }
        }
    }

    @Test
    func customEndpointsDoNotImpersonateAProvider() {
        #expect(ProviderKindWire.openAICompatible.logoAssetName == nil)
        #expect(!ProviderKindWire.openAICompatible.usesMonochromeLogo)
    }
}
