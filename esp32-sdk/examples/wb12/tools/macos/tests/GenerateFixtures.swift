import CoreGraphics
import CoreText
import Foundation
import ImageIO

private func writePNG(
    width: Int,
    height: Int,
    path: String,
    draw: (CGContext) -> Void
) throws {
    let colorSpace = CGColorSpaceCreateDeviceRGB()
    guard let context = CGContext(
        data: nil,
        width: width,
        height: height,
        bitsPerComponent: 8,
        bytesPerRow: width * 4,
        space: colorSpace,
        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else {
        throw NSError(domain: "GenerateFixtures", code: 1)
    }
    draw(context)
    guard let image = context.makeImage() else {
        throw NSError(domain: "GenerateFixtures", code: 2)
    }
    let url = URL(fileURLWithPath: path) as CFURL
    guard let destination = CGImageDestinationCreateWithURL(
        url,
        "public.png" as CFString,
        1,
        nil
    ) else {
        throw NSError(domain: "GenerateFixtures", code: 3)
    }
    CGImageDestinationAddImage(destination, image, nil)
    guard CGImageDestinationFinalize(destination) else {
        throw NSError(domain: "GenerateFixtures", code: 4)
    }
}

guard CommandLine.arguments.count == 4 else {
    FileHandle.standardError.write(
        Data("expected OCR, rectangle, and blank output paths\n".utf8)
    )
    exit(2)
}

try writePNG(width: 900, height: 240, path: CommandLine.arguments[1]) { context in
    context.setFillColor(CGColor(gray: 1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: 900, height: 240))

    let attributes: [NSAttributedString.Key: Any] = [
        NSAttributedString.Key(kCTFontAttributeName as String): CTFontCreateWithName(
            "Helvetica-Bold" as CFString,
            100,
            nil
        ),
        NSAttributedString.Key(kCTForegroundColorAttributeName as String): CGColor(
            gray: 0,
            alpha: 1
        ),
    ]
    let line = CTLineCreateWithAttributedString(
        NSAttributedString(string: "BOBE 1234", attributes: attributes)
    )
    context.textPosition = CGPoint(x: 80, y: 70)
    CTLineDraw(line, context)
}

try writePNG(width: 640, height: 480, path: CommandLine.arguments[2]) { context in
    context.setFillColor(CGColor(gray: 0.05, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: 640, height: 480))

    context.beginPath()
    context.move(to: CGPoint(x: 95, y: 75))
    context.addLine(to: CGPoint(x: 545, y: 105))
    context.addLine(to: CGPoint(x: 515, y: 405))
    context.addLine(to: CGPoint(x: 125, y: 385))
    context.closePath()
    context.setFillColor(CGColor(gray: 0.98, alpha: 1))
    context.fillPath()

    context.beginPath()
    context.move(to: CGPoint(x: 145, y: 130))
    context.addLine(to: CGPoint(x: 490, y: 150))
    context.addLine(to: CGPoint(x: 475, y: 355))
    context.addLine(to: CGPoint(x: 165, y: 340))
    context.closePath()
    context.setStrokeColor(CGColor(gray: 0.1, alpha: 1))
    context.setLineWidth(8)
    context.strokePath()
}

try writePNG(width: 640, height: 480, path: CommandLine.arguments[3]) { context in
    context.setFillColor(CGColor(gray: 0.5, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: 640, height: 480))
}
