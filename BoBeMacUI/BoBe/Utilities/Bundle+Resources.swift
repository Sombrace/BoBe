import Foundation

nonisolated extension Bundle {
    static var appResources: Bundle {
        #if SWIFT_PACKAGE
            return Bundle.module
        #else
            return Bundle.main
        #endif
    }
}
