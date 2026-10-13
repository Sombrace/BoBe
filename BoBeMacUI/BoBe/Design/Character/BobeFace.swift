import SwiftUI

@Animatable
struct BobeFace: View {
    var pose: BobePose
    var lid: Double = 0
    var voice: Double = 0
    @AnimatableIgnored var palette: BobeAvatarPalette

    var body: some View {
        Canvas { context, size in
            BobeFacePainter(pose: self.pose, lid: self.lid, voice: self.voice, palette: self.palette)
                .draw(in: &context, size: size)
        }
        .accessibilityHidden(true)
    }
}

struct BobeFacePainter {
    let pose: BobePose
    let lid: Double
    let voice: Double
    let palette: BobeAvatarPalette

    static let outlineRatio: CGFloat = 0.09
    static let mouthInterior = Color(rgb: 0x5A2B2A)
    static let tongue = Color(rgb: 0xF59AA5)

    private static func clamp(_ value: Double) -> Double {
        max(0, min(1, value))
    }

    func draw(in context: inout GraphicsContext, size: CGSize) {
        let side = min(size.width, size.height)
        let radius = side / 2 / (1 + Self.outlineRatio / 2)
        let center = CGPoint(x: size.width / 2, y: size.height / 2)
        self.drawBall(in: &context, center: center, radius: radius)

        var face = context
        face.translateBy(x: center.x, y: center.y)
        let eyeY = radius * 0.06
        let eyeX = radius * 0.37
        let lookX = self.pose.gazeX * radius * 0.055
        let lookY = self.pose.gazeY * radius * 0.05
        self.drawBlush(in: &face, radius: radius, eyeY: eyeY, lookX: lookX)
        self.drawEyes(in: &face, radius: radius, eyeX: eyeX, eyeY: eyeY, look: CGPoint(x: lookX, y: lookY))
        self.drawViewfinder(in: &face, radius: radius, eyeX: eyeX, eyeY: eyeY, lookX: lookX)
        self.drawMouth(in: &face, radius: radius, eyeY: eyeY, lookX: lookX)
    }

    private func drawBall(in context: inout GraphicsContext, center: CGPoint, radius: CGFloat) {
        let rect = CGRect(x: center.x - radius, y: center.y - radius, width: radius * 2, height: radius * 2)
        let ball = Path(ellipseIn: rect)
        context.fill(
            ball,
            with: .radialGradient(
                Gradient(colors: [self.palette.ballLight, self.palette.ballDark]),
                center: CGPoint(x: rect.minX + radius * 0.72, y: rect.minY + radius * 0.6),
                startRadius: 0,
                endRadius: radius * 1.64
            )
        )
        context.stroke(ball, with: .color(self.palette.outline), lineWidth: radius * Self.outlineRatio)

        var shine = context
        shine.translateBy(x: center.x - radius * 0.47, y: center.y - radius * 0.5)
        shine.rotate(by: .degrees(-42))
        shine.fill(
            Path(ellipseIn: CGRect(x: -radius * 0.17, y: -radius * 0.08, width: radius * 0.34, height: radius * 0.16)),
            with: .color(.white.opacity(0.8))
        )
        let dot = radius * 0.042
        context.fill(
            Path(ellipseIn: CGRect(
                x: center.x - radius * 0.2 - dot, y: center.y - radius * 0.7 - dot, width: dot * 2, height: dot * 2
            )),
            with: .color(.white.opacity(0.8))
        )
    }

    private func drawBlush(in context: inout GraphicsContext, radius: CGFloat, eyeY: CGFloat, lookX: CGFloat) {
        guard self.pose.blush > 0.01 else { return }
        for sign in [-1.0, 1.0] {
            let rect = CGRect(
                x: sign * radius * 0.62 + lookX * 0.3 - radius * 0.125,
                y: eyeY + radius * 0.2 - radius * 0.085,
                width: radius * 0.25,
                height: radius * 0.17
            )
            context.fill(Path(ellipseIn: rect), with: .color(self.palette.blush.opacity(Self.clamp(self.pose.blush) * 0.62)))
        }
    }

    private func drawEyes(
        in context: inout GraphicsContext, radius: CGFloat, eyeX: CGFloat, eyeY: CGFloat, look: CGPoint
    ) {
        let big = 1 + (self.pose.pupil - 1) * 0.6
        let open = Self.clamp(self.pose.open * (1 - self.lid))
        let eyeOpacity = Self.clamp((open - 0.03) / 0.2)
        let lidOpacity = 1 - Self.clamp((open - 0.08) / 0.26)
        for sign in [-1.0, 1.0] {
            let cx = sign * eyeX + look.x
            let cy = eyeY + look.y
            let rx = radius * 0.12 * self.pose.eyeWidth * big
            let fullRy = radius * 0.142 * self.pose.eyeHeight * big
            let ry = max(fullRy * open, 0.01)

            if self.pose.brow > 0.01 {
                let browY = cy - fullRy - radius * 0.085
                var brow = Path()
                brow.move(to: CGPoint(x: cx - rx, y: browY + radius * 0.012))
                brow.addQuadCurve(
                    to: CGPoint(x: cx + rx, y: browY + radius * 0.012),
                    control: CGPoint(x: cx, y: browY - radius * 0.06)
                )
                context.stroke(
                    brow,
                    with: .color(self.palette.line.opacity(Self.clamp(self.pose.brow) * 0.75)),
                    style: StrokeStyle(lineWidth: radius * 0.04, lineCap: .round)
                )
            }

            if eyeOpacity > 0 {
                let glint = min(1, open * 1.3)
                var eye = context
                eye.opacity = eyeOpacity
                eye.fill(
                    Path(ellipseIn: CGRect(x: cx - rx, y: cy - ry, width: rx * 2, height: ry * 2)),
                    with: .color(self.palette.eye)
                )
                let mainGlint = rx * 0.37 * glint
                eye.fill(
                    Path(ellipseIn: CGRect(
                        x: cx - rx * 0.3 - self.pose.gazeX * rx * 0.1 - mainGlint,
                        y: cy - ry * 0.36 - self.pose.gazeY * ry * 0.08 - mainGlint,
                        width: mainGlint * 2,
                        height: mainGlint * 2
                    )),
                    with: .color(.white)
                )
                let smallGlint = rx * 0.17 * glint
                eye.fill(
                    Path(ellipseIn: CGRect(
                        x: cx + rx * 0.36 - smallGlint, y: cy + ry * 0.4 - smallGlint,
                        width: smallGlint * 2, height: smallGlint * 2
                    )),
                    with: .color(.white.opacity(0.85))
                )
            }

            if lidOpacity > 0 {
                var closed = Path()
                closed.move(to: CGPoint(x: cx - rx * 1.08, y: cy))
                closed.addQuadCurve(
                    to: CGPoint(x: cx + rx * 1.08, y: cy),
                    control: CGPoint(x: cx, y: cy + self.pose.bend * rx * 1.05)
                )
                context.stroke(
                    closed,
                    with: .color(self.palette.eye.opacity(lidOpacity)),
                    style: StrokeStyle(lineWidth: radius * 0.05, lineCap: .round)
                )
            }
        }
    }

    private func drawViewfinder(
        in context: inout GraphicsContext, radius: CGFloat, eyeX: CGFloat, eyeY: CGFloat, lookX: CGFloat
    ) {
        guard self.pose.viewfinder > 0.01 else { return }
        let width = eyeX + radius * 0.25
        let height = radius * 0.26
        let corner = radius * 0.09
        var path = Path()
        for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let point = CGPoint(x: lookX + sx * width, y: eyeY + sy * height)
            path.move(to: CGPoint(x: point.x, y: point.y - sy * corner))
            path.addLine(to: point)
            path.addLine(to: CGPoint(x: point.x - sx * corner, y: point.y))
        }
        context.stroke(
            path,
            with: .color(self.palette.line.opacity(Self.clamp(self.pose.viewfinder) * 0.7)),
            style: StrokeStyle(lineWidth: radius * 0.03, lineCap: .round, lineJoin: .round)
        )
    }

    private func drawMouth(in context: inout GraphicsContext, radius: CGFloat, eyeY: CGFloat, lookX: CGFloat) {
        let mouthY = eyeY + radius * 0.2
        let openness = Self.clamp(max(self.pose.mouthOpen, self.voice))
        let width = radius * 0.09 * self.pose.mouthWidth
        let height = radius * 0.09 * self.pose.smile
        let drop = openness * radius * 0.19
        let x = lookX * 0.5
        let oh = Self.clamp(self.pose.oh)

        if oh < 0.99 {
            let top = height * (1 - 0.9 * openness) - drop * 0.3
            let bottom = height + drop * 1.5
            var mouth = Path()
            mouth.move(to: CGPoint(x: x - width, y: mouthY))
            mouth.addQuadCurve(to: CGPoint(x: x + width, y: mouthY), control: CGPoint(x: x, y: mouthY + top))
            mouth.addQuadCurve(to: CGPoint(x: x - width, y: mouthY), control: CGPoint(x: x, y: mouthY + bottom))
            mouth.closeSubpath()
            var layer = context
            layer.opacity = 1 - oh
            layer.fill(mouth, with: .color(Self.mouthInterior.opacity(Self.clamp(openness * 5))))
            layer.stroke(
                mouth,
                with: .color(self.palette.mouth),
                style: StrokeStyle(lineWidth: radius * 0.046, lineCap: .round, lineJoin: .round)
            )
            if openness > 0.4 {
                let tongueRect = CGRect(
                    x: x - width * 0.45, y: mouthY + bottom * 0.36 - drop * 0.16,
                    width: width * 0.9, height: drop * 0.32
                )
                layer.fill(
                    Path(ellipseIn: tongueRect),
                    with: .color(Self.tongue.opacity(Self.clamp((openness - 0.4) * 4) * 0.9))
                )
            }
        }

        if oh > 0.01 {
            let rx = radius * (0.042 + openness * 0.02)
            let ry = radius * (0.05 + openness * 0.05)
            let rect = CGRect(x: x - rx, y: mouthY + radius * 0.03 - ry, width: rx * 2, height: ry * 2)
            var layer = context
            layer.opacity = oh
            layer.fill(Path(ellipseIn: rect), with: .color(Self.mouthInterior))
            layer.stroke(Path(ellipseIn: rect), with: .color(self.palette.mouth), lineWidth: radius * 0.036)
        }
    }
}
