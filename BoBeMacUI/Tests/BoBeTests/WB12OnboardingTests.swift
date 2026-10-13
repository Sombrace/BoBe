@testable import BoBe
import Testing

@Suite("WB-12 onboarding")
struct WB12OnboardingTests {
    @Test
    func protocolUsesUnpaddedBase64URL() {
        #expect(
            WB12OnboardingProtocol.base64URL("WrongNetwork")
                == "V3JvbmdOZXR3b3Jr"
        )
    }

    @Test
    func identityRequiresMatchingNonceAndHardware() {
        let nonce = "0123456789abcdef0123456789abcdef"
        #expect(
            WB12OnboardingProtocol.identity(
                from: "@BOBE-WIFI/1 ID \(nonce) a1b2c3d4e5f6 WB-12",
                nonce: nonce
            ) == "a1b2c3d4e5f6"
        )
        #expect(
            WB12OnboardingProtocol.identity(
                from: "@BOBE-WIFI/1 ID \(nonce) e072a1f82bb4 PAPER",
                nonce: nonce
            ) == nil
        )
    }
}
