import Foundation

enum BargeInPolicy {
    static let echoGuardMilliseconds: UInt64 = 350
    static let duckedVolume: Float = 0.3
    static let unduckAfter: Duration = .milliseconds(700)
}
