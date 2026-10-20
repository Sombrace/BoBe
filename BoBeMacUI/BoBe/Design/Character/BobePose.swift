import SwiftUI

nonisolated struct BobePose: Equatable, Sendable, VectorArithmetic {
    private var storage: SIMD16<Double>

    init(
        open: Double = 1,
        eyeWidth: Double = 1,
        eyeHeight: Double = 1,
        gazeX: Double = 0,
        gazeY: Double = 0,
        pupil: Double = 1,
        brow: Double = 0,
        bend: Double = 0.25,
        smile: Double = 1,
        mouthWidth: Double = 1,
        mouthOpen: Double = 0,
        oh: Double = 0,
        blush: Double = 0.55,
        viewfinder: Double = 0
    ) {
        self.storage = SIMD16(
            open, eyeWidth, eyeHeight, gazeX, gazeY, pupil, brow, bend,
            smile, mouthWidth, mouthOpen, oh, blush, viewfinder, 0, 0
        )
    }

    private init(storage: SIMD16<Double>) {
        self.storage = storage
    }

    var open: Double {
        get { self.storage[0] }
        set { self.storage[0] = newValue }
    }

    var eyeWidth: Double {
        get { self.storage[1] }
        set { self.storage[1] = newValue }
    }

    var eyeHeight: Double {
        get { self.storage[2] }
        set { self.storage[2] = newValue }
    }

    var gazeX: Double {
        get { self.storage[3] }
        set { self.storage[3] = newValue }
    }

    var gazeY: Double {
        get { self.storage[4] }
        set { self.storage[4] = newValue }
    }

    var pupil: Double {
        get { self.storage[5] }
        set { self.storage[5] = newValue }
    }

    var brow: Double {
        get { self.storage[6] }
        set { self.storage[6] = newValue }
    }

    var bend: Double {
        get { self.storage[7] }
        set { self.storage[7] = newValue }
    }

    var smile: Double {
        get { self.storage[8] }
        set { self.storage[8] = newValue }
    }

    var mouthWidth: Double {
        get { self.storage[9] }
        set { self.storage[9] = newValue }
    }

    var mouthOpen: Double {
        get { self.storage[10] }
        set { self.storage[10] = newValue }
    }

    var oh: Double {
        get { self.storage[11] }
        set { self.storage[11] = newValue }
    }

    var blush: Double {
        get { self.storage[12] }
        set { self.storage[12] = newValue }
    }

    var viewfinder: Double {
        get { self.storage[13] }
        set { self.storage[13] = newValue }
    }

    static var zero: BobePose {
        BobePose(storage: .zero)
    }

    static func + (lhs: BobePose, rhs: BobePose) -> BobePose {
        BobePose(storage: lhs.storage + rhs.storage)
    }

    static func - (lhs: BobePose, rhs: BobePose) -> BobePose {
        BobePose(storage: lhs.storage - rhs.storage)
    }

    mutating func scale(by rhs: Double) {
        self.storage *= rhs
    }

    var magnitudeSquared: Double {
        (self.storage * self.storage).sum()
    }

    func looking(x: Double, y: Double) -> BobePose {
        var pose = self
        pose.gazeX = max(-1, min(1, pose.gazeX + x))
        pose.gazeY = max(-1, min(1, pose.gazeY + y))
        return pose
    }
}

nonisolated extension BobePose {
    static let idle = BobePose()
    static let asleep = BobePose(open: 0, bend: 1, smile: 0.6, mouthWidth: 0.85, blush: 0.7)
    static let waking = BobePose(open: 0.55, eyeHeight: 0.9, smile: 0.8, blush: 0.6)
    static let happy = BobePose(open: 0, bend: -1, smile: 1.45, mouthWidth: 1.25, blush: 1)
    static let listening = BobePose(pupil: 1.14, brow: 0.55, smile: 0.9, mouthWidth: 0.9, blush: 0.7)
    static let thinking = BobePose(
        eyeHeight: 0.92, gazeX: 0.55, gazeY: -0.85, brow: 0.5, smile: 0.35, mouthWidth: 0.7, blush: 0.4
    )
    static let working = BobePose(
        eyeHeight: 0.86, gazeX: -0.15, gazeY: 0.9, brow: 0.85, smile: 0.55, mouthWidth: 0.8, blush: 0.45
    )
    static let speaking = BobePose(pupil: 1.04, smile: 1.1, mouthWidth: 1.1, blush: 0.75)
    static let eager = BobePose(gazeY: -0.1, pupil: 1.22, brow: 1, smile: 1.35, mouthWidth: 1.15, blush: 0.9)
    static let watching = BobePose(
        eyeWidth: 0.94, eyeHeight: 0.92, gazeX: -0.8, smile: 0.5, mouthWidth: 0.85, blush: 0.3, viewfinder: 1
    )
    static let worried = BobePose(
        eyeHeight: 0.9, gazeY: 0.35, pupil: 0.94, brow: 0.2, smile: -0.7, mouthWidth: 0.8, blush: 0.15
    )
}
