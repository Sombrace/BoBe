import AppKit
import Observation

@MainActor
@Observable
final class PointerTracker {
    static let shared = PointerTracker()

    private(set) var location: CGPoint = NSEvent.mouseLocation

    @ObservationIgnored private var monitors: [Any] = []
    @ObservationIgnored private var clients = 0

    private static let minimumTravel: CGFloat = 5

    private init() {}

    func retain() {
        self.clients += 1
        guard self.clients == 1 else { return }
        self.location = NSEvent.mouseLocation
        let mask: NSEvent.EventTypeMask = [.mouseMoved, .leftMouseDragged]
        if let global = NSEvent.addGlobalMonitorForEvents(matching: mask, handler: { [weak self] _ in
            MainActor.assumeIsolated { self?.publish() }
        }) {
            self.monitors.append(global)
        }
        if let local = NSEvent.addLocalMonitorForEvents(matching: mask, handler: { [weak self] event in
            MainActor.assumeIsolated { self?.publish() }
            return event
        }) {
            self.monitors.append(local)
        }
    }

    func release() {
        guard self.clients > 0 else { return }
        self.clients -= 1
        guard self.clients == 0 else { return }
        for monitor in self.monitors {
            NSEvent.removeMonitor(monitor)
        }
        self.monitors.removeAll()
    }

    private func publish() {
        let current = NSEvent.mouseLocation
        guard hypot(current.x - self.location.x, current.y - self.location.y) >= Self.minimumTravel else { return }
        self.location = current
    }

    static func gaze(from center: CGPoint, to pointer: CGPoint, reach: CGFloat = 240) -> CGSize {
        let dx = pointer.x - center.x
        let dy = pointer.y - center.y
        let distance = hypot(dx, dy)
        guard distance > 1 else { return .zero }
        let strength = min(1, distance / reach)
        let quantize: (CGFloat) -> CGFloat = { ($0 * 8).rounded() / 8 }
        return CGSize(width: quantize(dx / distance * strength), height: quantize(-dy / distance * strength))
    }
}
