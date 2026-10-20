@testable import BoBe
import AppKit
import Testing

@Suite("Provider brand assets")
struct ProviderBrandAssetTests {
    @Test(arguments: [NSAppearance.Name.aqua, .darkAqua], ProviderKindWire.allCases.filter { $0 != .openAICompatible })
    func providerLogosLoadFromTheAppBundleInBothAppearances(mode: NSAppearance.Name, kind: ProviderKindWire) throws {
        let appearance = try #require(NSAppearance(named: mode))
        let name = try #require(kind.logoAssetName)
        var loaded: NSImage?
        appearance.performAsCurrentDrawingAppearance {
            loaded = Bundle.appResources.image(forResource: NSImage.Name(name))
        }
        let image = try #require(loaded)
        #expect(image.size.width > 0 && image.size.height > 0)
        #expect(image.isTemplate == kind.usesMonochromeLogo)
    }

    @Test
    func customEndpointsDoNotImpersonateAProvider() {
        #expect(ProviderKindWire.openAICompatible.logoAssetName == nil)
        #expect(!ProviderKindWire.openAICompatible.usesMonochromeLogo)
    }
}
