import AVFoundation
import SwiftUI
import UIKit

struct CodeScanner: UIViewRepresentable {
    let onCode: @MainActor (String) -> Void

    static var isAvailable: Bool {
        AVCaptureDevice.default(for: .video) != nil
    }

    func makeUIView(context: Context) -> ScannerView {
        let view = ScannerView()
        view.onCode = self.onCode
        view.start()
        return view
    }

    func updateUIView(_ view: ScannerView, context: Context) {
        view.onCode = self.onCode
    }

    static func dismantleUIView(_ view: ScannerView, coordinator: ()) {
        view.stop()
    }
}

final class ScannerView: UIView, AVCaptureMetadataOutputObjectsDelegate {
    var onCode: (@MainActor (String) -> Void)?
    private let capture = CaptureBox()
    private var lastCode: String?

    override static var layerClass: AnyClass {
        AVCaptureVideoPreviewLayer.self
    }

    func start() {
        guard let preview = self.layer as? AVCaptureVideoPreviewLayer,
              let device = AVCaptureDevice.default(for: .video),
              let input = try? AVCaptureDeviceInput(device: device)
        else { return }
        let session = self.capture.session
        guard session.canAddInput(input) else { return }
        session.addInput(input)
        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else { return }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        output.metadataObjectTypes = [.qr]
        preview.session = session
        preview.videoGravity = .resizeAspectFill
        self.capture.run()
    }

    func stop() {
        self.capture.halt()
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        guard let value = (metadataObjects.first as? AVMetadataMachineReadableCodeObject)?.stringValue else { return }
        self.found(value)
    }

    private func found(_ value: String) {
        guard value != self.lastCode else { return }
        self.lastCode = value
        UINotificationFeedbackGenerator(view: self).notificationOccurred(.success)
        self.onCode?(value)
    }
}

nonisolated private final class CaptureBox: @unchecked Sendable {
    let session = AVCaptureSession()
    private let queue = DispatchQueue(label: "com.bobe.ios.scanner")

    func run() {
        self.queue.async { self.session.startRunning() }
    }

    func halt() {
        self.queue.async { self.session.stopRunning() }
    }
}

struct ScanBrackets: View {
    @Environment(\.theme) private var theme
    @State private var breathe = false

    var body: some View {
        GeometryReader { proxy in
            let inset = proxy.size.width * 0.14
            let arm = proxy.size.width * 0.12
            Path { path in
                let rect = CGRect(origin: .zero, size: proxy.size).insetBy(dx: inset, dy: inset)
                for corner in [
                    (rect.minX, rect.minY, 1.0, 1.0), (rect.maxX, rect.minY, -1.0, 1.0),
                    (rect.minX, rect.maxY, 1.0, -1.0), (rect.maxX, rect.maxY, -1.0, -1.0),
                ] {
                    path.move(to: CGPoint(x: corner.0, y: corner.1 + arm * corner.3))
                    path.addLine(to: CGPoint(x: corner.0, y: corner.1))
                    path.addLine(to: CGPoint(x: corner.0 + arm * corner.2, y: corner.1))
                }
            }
            .stroke(self.theme.glow[1], style: StrokeStyle(lineWidth: 4, lineCap: .round, lineJoin: .round))
            .scaleEffect(self.breathe ? 1.03 : 0.98)
            .shadow(color: self.theme.glow[1].opacity(0.5), radius: 6)
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        .task {
            guard !BobeMotion.isReduced else { return }
            withAnimation(.easeInOut(duration: 1.6).repeatForever(autoreverses: true)) {
                self.breathe = true
            }
        }
    }
}
