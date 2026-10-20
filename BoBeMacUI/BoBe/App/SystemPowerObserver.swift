import AppKit
import OSLog

nonisolated private let logger = Logger(subsystem: "com.bobe.app", category: "SystemPower")

enum SurfacePowerState: Equatable {
    case stopped
    case running
    case sleeping
    case terminating

    mutating func prepareForSleep() -> Bool {
        guard self == .running else { return false }
        self = .sleeping
        return true
    }

    mutating func resumeAfterWake() -> Bool {
        guard self == .sleeping else { return false }
        self = .running
        return true
    }
}

final class SystemPowerObserver {
    static let shared = SystemPowerObserver()

    private var sleepObserver: NSObjectProtocol?
    private var wakeObserver: NSObjectProtocol?
    private(set) var state: SurfacePowerState = .stopped

    private init() {}

    func start() {
        guard self.state != .terminating else { return }
        self.state = .running
        let center = NSWorkspace.shared.notificationCenter
        if self.sleepObserver == nil {
            self.sleepObserver = center.addObserver(
                forName: NSWorkspace.willSleepNotification,
                object: nil,
                queue: .main
            ) { _ in
                Task { @MainActor in
                    await SystemPowerObserver.shared.prepareForSleep()
                }
            }
        }
        if self.wakeObserver == nil {
            self.wakeObserver = center.addObserver(
                forName: NSWorkspace.didWakeNotification,
                object: nil,
                queue: .main
            ) { _ in
                Task { @MainActor in
                    await SystemPowerObserver.shared.resumeAfterWake()
                }
            }
        }
    }

    func prepareForSleep() async {
        guard self.state.prepareForSleep() else { return }
        logger.info("System will sleep — suspending voice and event stream")
        VoicePipeline.shared.disconnect()
        await BobeStore.shared.suspendEventStream()
    }

    func resumeAfterWake() async {
        guard self.state.resumeAfterWake() else { return }
        logger.info("System did wake — restoring runtime and event stream")

        let isManagedDevelopment = DaemonConfig.endpoint.managesLocalProcess
            && ProcessInfo.processInfo.environment["BOBE_DEV"] != nil
        if !isManagedDevelopment {
            do {
                try await BackendService.shared.start()
            } catch {
                logger.warning(
                    "Runtime health restoration after wake failed: \(error.localizedDescription, privacy: .public)"
                )
            }
        }

        do {
            _ = try await EndpointAttachService.shared.attachConfiguredEndpoint()
        } catch {
            logger.warning(
                "Runtime attach verification after wake failed: \(error.localizedDescription, privacy: .public)"
            )
        }

        if SetupWindowManager.shared.isOnboardingCompleted {
            BobeStore.shared.resumeEventStream()
        }
        await VoicePipeline.shared.refreshDaemonState()
        VoicePipeline.shared.bootstrapSttPresence()
    }

    func stop() {
        self.state = .terminating
        let center = NSWorkspace.shared.notificationCenter
        if let sleepObserver = self.sleepObserver {
            center.removeObserver(sleepObserver)
            self.sleepObserver = nil
        }
        if let wakeObserver = self.wakeObserver {
            center.removeObserver(wakeObserver)
            self.wakeObserver = nil
        }
    }
}
