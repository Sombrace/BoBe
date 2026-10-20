import SwiftUI

enum BobeMood: Equatable, Sendable {
    case asleep
    case waking
    case idle
    case listening
    case thinking
    case working
    case speaking
    case eager
    case happy
    case watching
    case worried

    var pose: BobePose {
        switch self {
        case .asleep: .asleep
        case .waking: .waking
        case .idle: .idle
        case .listening: .listening
        case .thinking: .thinking
        case .working: .working
        case .speaking: .speaking
        case .eager: .eager
        case .happy: .happy
        case .watching: .watching
        case .worried: .worried
        }
    }

    var blinks: Bool {
        switch self {
        case .asleep, .happy: false
        default: true
        }
    }

    var followsPointer: Bool {
        switch self {
        case .idle, .listening, .speaking, .eager: true
        default: false
        }
    }

    var breathes: Bool {
        switch self {
        case .asleep, .idle, .eager: true
        default: false
        }
    }

    var orbitPeriod: Double? {
        switch self {
        case .thinking: 2.2
        case .working: 1.5
        case .waking: 3.2
        default: nil
        }
    }

    var hearsVoice: Bool {
        self == .listening || self == .speaking
    }

    var restAngle: Double {
        switch self {
        case .listening: 152
        case .eager, .happy: 315
        case .asleep: 62
        case .worried: 110
        default: 38
        }
    }

    var squashKick: Double {
        switch self {
        case .asleep: -0.03
        case .happy: 0.09
        case .eager: 0.08
        default: 0.055
        }
    }

    var hopKick: Double {
        switch self {
        case .happy: 0.085
        case .eager: 0.06
        default: 0
        }
    }

    var ringStrength: Double {
        switch self {
        case .worried: 0.5
        case .thinking, .working, .waking: 0.24
        default: 1
        }
    }
}

enum BobeCharacterMetrics {
    static let ballRatio: CGFloat = 0.78
    static let ringWidthRatio: CGFloat = 0.05
    static let moonRatio: CGFloat = 0.08
    static let frameRatio: CGFloat = 1.2

    static func frameSide(for size: CGFloat) -> CGFloat {
        ceil(size * self.frameRatio)
    }
}

struct BobeCharacter: View {
    var mood: BobeMood
    var size: CGFloat
    let palette: BobeAvatarPalette
    var gaze: CGSize = .zero
    var voiceLevel: Double = 0
    var mouthLevel: Double = 0
    var wantsAttention = false
    var progress: Double?
    var isLively = true

    @State private var lid = 0.0
    @State private var glance = CGSize.zero
    @State private var breathIn = false
    @State private var moonAngle = BobeMood.idle.restAngle
    @State private var orbitStart: Date?
    @State private var orbitPeriod = 2.2
    @State private var attentionPulse = false
    @State private var tilt = 0.0
    @State private var cheer = 0

    private var radius: CGFloat { self.size / 2 }
    private var ringWidth: CGFloat { self.size * BobeCharacterMetrics.ringWidthRatio }
    private var moonDiameter: CGFloat { self.size * BobeCharacterMetrics.moonRatio * 2 }
    private var ballSide: CGFloat {
        self.size * BobeCharacterMetrics.ballRatio * (1 + BobeFacePainter.outlineRatio / 2)
    }

    private var animates: Bool {
        self.isLively && !BobeMotion.isReduced
    }

    private var effectivePose: BobePose {
        let look = self.mood.followsPointer ? self.gaze : .zero
        return self.mood.pose.looking(
            x: Double(look.width + self.glance.width),
            y: Double(look.height + self.glance.height)
        )
    }

    var body: some View {
        ZStack {
            self.ring
            self.moonLayer
            self.ball
            self.cheerLayer
        }
        .frame(
            width: BobeCharacterMetrics.frameSide(for: self.size),
            height: BobeCharacterMetrics.frameSide(for: self.size)
        )
        .keyframeAnimator(initialValue: 0.0, trigger: self.mood) { content, hop in
            content.offset(y: -hop * self.size)
        } keyframes: { _ in
            let kick = self.animates ? self.mood.hopKick : 0
            KeyframeTrack {
                CubicKeyframe(-kick * 0.14, duration: 0.08)
                CubicKeyframe(kick, duration: 0.15)
                CubicKeyframe(0.0, duration: 0.17)
                SpringKeyframe(0.0, duration: 0.24, spring: Spring(response: 0.2, dampingRatio: 0.78))
            }
        }
        .onChange(of: self.mood, initial: true) { old, new in
            self.moodChanged(from: old, to: new)
            if new == .happy, old != .happy, self.animates {
                self.cheer &+= 1
            }
        }
        .task(id: self.mood.blinks && self.animates) {
            await self.liveFace()
        }
        .task(id: BusyFaceKey(mood: self.mood, animates: self.animates)) {
            await self.busyFace()
        }
        .task(id: self.mood.breathes && self.animates) {
            await self.breathe()
        }
        .task(id: self.wantsAttention && self.animates) {
            self.attentionPulse = false
            guard self.wantsAttention, self.animates else { return }
            withAnimation(.easeOut(duration: 1.4).repeatForever(autoreverses: false)) {
                self.attentionPulse = true
            }
        }
        .accessibilityHidden(true)
    }

    @ViewBuilder
    private var ring: some View {
        if let progress {
            ZStack {
                Circle().stroke(self.palette.orbit.opacity(0.22), lineWidth: self.ringWidth)
                Circle()
                    .trim(from: 0, to: max(0.001, min(1, progress)))
                    .stroke(self.palette.orbit, style: StrokeStyle(lineWidth: self.ringWidth, lineCap: .round))
                    .rotationEffect(.degrees(-90))
            }
            .frame(width: self.size, height: self.size)
        } else if self.mood.hearsVoice, self.animates {
            TimelineView(.animation) { timeline in
                BobeVoiceRing(
                    phase: timeline.date.timeIntervalSinceReferenceDate,
                    level: max(0.08, min(1, self.voiceLevel))
                )
                .stroke(self.palette.orbit, style: StrokeStyle(lineWidth: self.ringWidth, lineJoin: .round))
            }
            .frame(width: self.size, height: self.size)
        } else {
            ZStack {
                Circle()
                    .stroke(
                        self.palette.orbit.opacity(self.animates ? self.mood.ringStrength : (self.mood == .worried ? 0.5 : 1)),
                        style: StrokeStyle(
                            lineWidth: self.ringWidth,
                            lineCap: .round,
                            dash: self.mood == .worried ? [self.ringWidth * 0.4, self.ringWidth * 2.2] : []
                        )
                    )
                    .animation(BobeMotion.soft, value: self.mood.ringStrength)
                if self.mood == .working, self.animates {
                    TimelineView(.animation) { timeline in
                        Circle()
                            .stroke(
                                self.palette.orbit.opacity(0.6),
                                style: StrokeStyle(
                                    lineWidth: self.ringWidth,
                                    lineCap: .round,
                                    dash: [self.ringWidth * 0.5, self.ringWidth * 2.4]
                                )
                            )
                            .rotationEffect(.degrees(-timeline.date.timeIntervalSinceReferenceDate * 42))
                    }
                    .transition(.opacity)
                }
            }
            .frame(width: self.size, height: self.size)
            .animation(BobeMotion.soft, value: self.mood == .working)
        }
    }

    @ViewBuilder
    private var moonLayer: some View {
        if let progress {
            self.moon(angle: -90 + 360 * max(0, min(1, progress)), trail: 0)
        } else if let orbitStart, self.animates {
            TimelineView(.animation) { timeline in
                let elapsed = timeline.date.timeIntervalSince(orbitStart)
                self.moon(
                    angle: self.moonAngle + Self.orbitTravel(elapsed, period: self.orbitPeriod),
                    trail: Self.orbitSpeedFraction(elapsed)
                )
            }
        } else {
            self.moon(angle: self.moonAngle, trail: 0)
        }
    }

    private func moon(angle: Double, trail: Double) -> some View {
        ZStack {
            if trail > 0.01 {
                Circle()
                    .trim(from: 0, to: Self.cometSpan / 360)
                    .stroke(
                        AngularGradient(
                            colors: [self.palette.orbit.opacity(0), self.palette.orbit.opacity(trail)],
                            center: .center,
                            startAngle: .degrees(0),
                            endAngle: .degrees(Self.cometSpan)
                        ),
                        style: StrokeStyle(lineWidth: self.ringWidth, lineCap: .round)
                    )
                    .frame(width: self.size, height: self.size)
                    .rotationEffect(.degrees(angle - Self.cometSpan))
            }
            ZStack {
                if self.wantsAttention {
                    Circle()
                        .stroke(self.palette.orbit, lineWidth: 1.5)
                        .frame(width: self.moonDiameter, height: self.moonDiameter)
                        .scaleEffect(self.attentionPulse ? 2.6 : 1)
                        .opacity(self.attentionPulse ? 0 : 0.7)
                }
                Circle()
                    .fill(self.wantsAttention ? self.palette.orbit : self.palette.moon)
                    .overlay {
                        Circle().strokeBorder(.white.opacity(self.wantsAttention ? 0.85 : 0), lineWidth: 1.5)
                    }
                    .opacity(self.wantsAttention ? 1 : 0.9)
                    .frame(width: self.moonDiameter, height: self.moonDiameter)
                    .scaleEffect(self.wantsAttention ? 1.25 : 1)
            }
            .offset(x: self.radius)
            .rotationEffect(.degrees(angle))
        }
        .animation(BobeMotion.body, value: self.wantsAttention)
    }

    private var ball: some View {
        BobeFace(pose: self.effectivePose, lid: self.lid, voice: self.mouthLevel, palette: self.palette)
            .frame(width: self.ballSide, height: self.ballSide)
            .shadow(color: self.palette.outline.opacity(0.18), radius: self.size * 0.035, y: self.size * 0.022)
            .scaleEffect(self.breathScale)
            .rotationEffect(.degrees(self.tilt))
            .keyframeAnimator(initialValue: 0.0, trigger: self.mood) { content, squash in
                content.scaleEffect(x: 1 + squash * 0.55, y: 1 - squash, anchor: .bottom)
            } keyframes: { _ in
                let kick = self.animates ? self.mood.squashKick : 0
                let lands = self.mood.hopKick > 0
                KeyframeTrack {
                    CubicKeyframe(kick, duration: 0.08)
                    CubicKeyframe(-kick * 0.55, duration: 0.15)
                    CubicKeyframe(lands ? kick * 0.7 : 0, duration: 0.17)
                    SpringKeyframe(0.0, duration: 0.3, spring: Spring(response: 0.22, dampingRatio: 0.74))
                }
            }
            .animation(BobeMotion.face, value: self.mood)
            .animation(BobeMotion.gaze, value: self.gaze)
            .animation(BobeMotion.gaze, value: self.glance)
            .animation(self.animates ? .linear(duration: 0.08) : nil, value: self.mouthLevel)
    }

    private static let cometSpan = 150.0

    @ViewBuilder
    private var cheerLayer: some View {
        if self.animates {
            let radius = self.radius
            let ringWidth = self.ringWidth
            let palette = self.palette
            Color.clear
                .keyframeAnimator(initialValue: 1.0, trigger: self.cheer) { _, progress in
                    BobeCheer(progress: progress, radius: radius, ringWidth: ringWidth, palette: palette)
                } keyframes: { _ in
                    KeyframeTrack {
                        MoveKeyframe(0.0)
                        CubicKeyframe(1.0, duration: 0.72, startVelocity: 3.2, endVelocity: 0)
                    }
                }
                .allowsHitTesting(false)
        }
    }

    private var breathScale: CGFloat {
        guard self.mood.breathes, self.animates else { return 1 }
        return self.breathIn ? 1.014 : 0.992
    }

    private func moodChanged(from old: BobeMood, to new: BobeMood) {
        if let period = new.orbitPeriod, self.animates {
            guard self.orbitStart == nil else {
                self.retime(to: period)
                return
            }
            var transaction = Transaction()
            transaction.disablesAnimations = true
            withTransaction(transaction) {
                self.moonAngle = self.moonAngle.truncatingRemainder(dividingBy: 360)
                self.orbitPeriod = period
                self.orbitStart = .now
            }
            return
        }
        guard let orbitStart else {
            guard old != new else {
                self.moonAngle = new.restAngle
                return
            }
            withAnimation(BobeMotion.soft) {
                self.moonAngle = Self.nearest(new.restAngle, to: self.moonAngle)
            }
            return
        }
        let elapsed = Date.now.timeIntervalSince(orbitStart)
        let current = self.moonAngle + Self.orbitTravel(elapsed, period: self.orbitPeriod)
        let speed = 360 / self.orbitPeriod * Self.orbitSpeedFraction(elapsed)
        var target = Self.nearest(new.restAngle, to: current)
        while target < current + 50 { target += 360 }
        var transaction = Transaction()
        transaction.disablesAnimations = true
        withTransaction(transaction) {
            self.moonAngle = current
            self.orbitStart = nil
        }
        withAnimation(
            .interpolatingSpring(mass: 1, stiffness: 46, damping: 11, initialVelocity: speed / (target - current))
        ) {
            self.moonAngle = target
        }
    }

    private func retime(to period: Double) {
        guard let orbitStart, abs(period - self.orbitPeriod) > 0.01 else { return }
        let elapsed = Date.now.timeIntervalSince(orbitStart)
        var transaction = Transaction()
        transaction.disablesAnimations = true
        withTransaction(transaction) {
            self.moonAngle = (self.moonAngle + Self.orbitTravel(elapsed, period: self.orbitPeriod))
                .truncatingRemainder(dividingBy: 360)
            self.orbitPeriod = period
            self.orbitStart = .now
        }
    }

    private static let orbitRamp = 0.35

    private static func orbitTravel(_ elapsed: Double, period: Double) -> Double {
        let time = max(0, elapsed)
        return 360 / period * (time - self.orbitRamp * (1 - exp(-time / self.orbitRamp)))
    }

    private static func orbitSpeedFraction(_ elapsed: Double) -> Double {
        1 - exp(-max(0, elapsed) / self.orbitRamp)
    }

    private static func nearest(_ angle: Double, to reference: Double) -> Double {
        let delta = (angle - reference).truncatingRemainder(dividingBy: 360)
        let shortest = delta > 180 ? delta - 360 : (delta < -180 ? delta + 360 : delta)
        return reference + shortest
    }

    private func liveFace() async {
        var transaction = Transaction()
        transaction.disablesAnimations = true
        withTransaction(transaction) {
            self.lid = 0
            self.glance = .zero
        }
        guard self.mood.blinks, self.animates else { return }
        do {
            while !Task.isCancelled {
                try await Task.sleep(for: .seconds(Double.random(in: 2.6 ... 6.4)))
                let roll = Double.random(in: 0 ..< 1)
                if roll < 0.16, self.gaze == .zero, self.mood == .idle {
                    self.glance = CGSize(width: Bool.random() ? 0.8 : -0.8, height: Double.random(in: -0.3 ... 0.2))
                    try await Task.sleep(for: .seconds(Double.random(in: 0.7 ... 1.3)))
                    self.glance = .zero
                    continue
                }
                try await self.blinkOnce()
                if roll > 0.84 {
                    try await Task.sleep(for: .milliseconds(120))
                    try await self.blinkOnce()
                }
            }
        } catch {
            withTransaction(transaction) {
                self.lid = 0
                self.glance = .zero
            }
        }
    }

    private func busyFace() async {
        if self.tilt != 0 {
            withAnimation(BobeMotion.face) { self.tilt = 0 }
        }
        guard self.animates else { return }
        switch self.mood {
        case .thinking:
            var left = false
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(Double.random(in: 1.3 ... 2.3)))
                guard !Task.isCancelled else { break }
                left.toggle()
                self.glance = left ? CGSize(width: -1.1, height: 0.12) : .zero
                withAnimation(BobeMotion.face) { self.tilt = left ? -5 : 4 }
            }
        case .working:
            var right = false
            while !Task.isCancelled {
                right.toggle()
                self.glance = CGSize(width: right ? 0.7 : -0.4, height: 0)
                try? await Task.sleep(for: .milliseconds(right ? 760 : 420))
            }
        default:
            return
        }
        self.glance = .zero
        withAnimation(BobeMotion.face) { self.tilt = 0 }
    }

    private func blinkOnce() async throws {
        withAnimation(.easeIn(duration: 0.09)) { self.lid = 1 }
        try await Task.sleep(for: .milliseconds(135))
        withAnimation(.easeOut(duration: 0.14)) { self.lid = 0 }
        try await Task.sleep(for: .milliseconds(140))
    }

    private func breathe() async {
        var transaction = Transaction()
        transaction.disablesAnimations = true
        withTransaction(transaction) { self.breathIn = false }
        guard self.mood.breathes, self.animates else { return }
        let period = self.mood == .asleep ? 3.6 : 2.8
        while !Task.isCancelled {
            withAnimation(.easeInOut(duration: period)) { self.breathIn.toggle() }
            try? await Task.sleep(for: .seconds(period + 0.2))
        }
    }
}

nonisolated struct BobeVoiceRing: Shape {
    var phase: Double
    var level: Double

    var animatableData: AnimatablePair<Double, Double> {
        get { AnimatablePair(self.phase, self.level) }
        set {
            self.phase = newValue.first
            self.level = newValue.second
        }
    }

    private static let segments = 96

    func path(in rect: CGRect) -> Path {
        var path = Path()
        let center = CGPoint(x: rect.midX, y: rect.midY)
        let base = Double(min(rect.width, rect.height) / 2)
        let low = base * (0.006 + 0.016 * self.level)
        let mid = base * 0.05 * self.level
        let high = base * 0.055 * self.level * self.level
        for index in 0 ... Self.segments {
            let theta = Double(index) / Double(Self.segments) * 2 * .pi
            let radius = base
                + low * sin(2 * theta + 0.5 * self.phase)
                + mid * sin(5 * theta + 1.3 * self.phase + 1.7)
                + high * sin(11 * theta + 2.4 * self.phase + 3.1)
            let point = CGPoint(
                x: center.x + CGFloat(radius * cos(theta)),
                y: center.y + CGFloat(radius * sin(theta))
            )
            if index == 0 {
                path.move(to: point)
            } else {
                path.addLine(to: point)
            }
        }
        path.closeSubpath()
        return path
    }
}

private struct BusyFaceKey: Hashable {
    let mood: BobeMood
    let animates: Bool
}

private struct BobeCheer: View {
    let progress: Double
    let radius: CGFloat
    let ringWidth: CGFloat
    let palette: BobeAvatarPalette

    private static let rays = 7

    var body: some View {
        let fade = max(0, 1 - self.progress)
        ZStack {
            Circle()
                .stroke(self.palette.orbit.opacity(0.55 * fade), lineWidth: max(0.5, self.ringWidth * fade))
                .frame(width: self.radius * 2, height: self.radius * 2)
                .scaleEffect(1 + 0.2 * self.progress)
            ForEach(0 ..< Self.rays, id: \.self) { index in
                let angle = Double(index) / Double(Self.rays) * 2 * .pi - .pi / 2 + 0.3
                let reach = self.radius * (1.06 + 0.3 * self.progress)
                let side = self.ringWidth * (index.isMultiple(of: 2) ? 1.5 : 1.05) * sin(.pi * min(1, self.progress * 1.15))
                BobeSpark()
                    .fill(index.isMultiple(of: 2) ? self.palette.orbit : self.palette.moon)
                    .frame(width: side, height: side)
                    .rotationEffect(.degrees(self.progress * 50))
                    .offset(x: reach * cos(angle), y: reach * sin(angle))
                    .opacity(min(1, fade * 1.6))
            }
        }
        .opacity(self.progress >= 1 ? 0 : 1)
    }
}

nonisolated private struct BobeSpark: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        let center = CGPoint(x: rect.midX, y: rect.midY)
        let outer = min(rect.width, rect.height) / 2
        let inner = outer * 0.34
        for index in 0 ..< 8 {
            let angle = Double(index) * .pi / 4 - .pi / 2
            let reach = index.isMultiple(of: 2) ? outer : inner
            let point = CGPoint(x: center.x + reach * cos(angle), y: center.y + reach * sin(angle))
            if index == 0 {
                path.move(to: point)
            } else {
                path.addLine(to: point)
            }
        }
        path.closeSubpath()
        return path
    }
}
