import Darwin
import Foundation
import os
import OSLog
import Security

private let logger = Logger(subsystem: "com.bobe.app", category: "BackendService")

enum ServiceState: Sendable {
    case stopped, starting, ready, crashed, fatal
}

private enum ManagedProcessIdentity {
    case verifiedDaemon(path: String)
    case otherProcess(path: String)
    case notRunning
    case unverifiable
}

private let terminateGrace: Duration = .seconds(45)
private let healthCheckInitialDelay: TimeInterval = 0.2
private let healthCheckBackoffMultiplier: Double = 1.5
private let healthCheckMaxDelay: TimeInterval = 5.0
private let healthCheckMaxAttempts = 30
private let stderrBufferMaxLines = 50

actor BackendService {
    static let shared = BackendService()

    private var process: Process?
    private var state: ServiceState = .stopped
    private var stopping = false
    private var restartCount = 0
    private var readySince: ContinuousClock.Instant?
    private var lifecycleGeneration: UInt64 = 0
    private var restartTask: Task<Void, Never>?
    private var stopTask: Task<Void, Never>?
    private var lastStartManagedLocal: Bool?
    private let maxRestartAttempts = 3
    private let dataDir: URL
    private let pidFilePath: URL
    private(set) var lastError: String?
    private(set) var startupWarning: String?
    private var stateContinuations: [UUID: AsyncStream<ServiceState>.Continuation] = [:]

    private init() {
        self.dataDir = DaemonConfig.managedDataDirectory
        self.pidFilePath = self.dataDir.appendingPathComponent("bobe-service.pid")
    }

    var stateStream: AsyncStream<ServiceState> {
        let (stream, continuation) = AsyncStream.makeStream(of: ServiceState.self)
        let id = UUID()
        self.stateContinuations[id] = continuation
        continuation.yield(self.state)
        continuation.onTermination = { @Sendable _ in
            Task { await self.unregisterContinuation(id: id) }
        }
        return stream
    }

    private func unregisterContinuation(id: UUID) {
        self.stateContinuations.removeValue(forKey: id)
    }

    private func transition(to newState: ServiceState) {
        self.state = newState
        for cont in self.stateContinuations.values {
            cont.yield(newState)
        }
    }

    func start() async throws {
        if let stopTask { await stopTask.value }
        let selection = EndpointStore.shared.selection
        let managesLocalProcess = DaemonConfig.endpoint.managesLocalProcess
        if let lastStartManagedLocal, lastStartManagedLocal != managesLocalProcess {
            await self.stop()
        }
        guard EndpointStore.shared.selection == selection else {
            throw BackendServiceError.stoppedDuringHealthCheck
        }
        self.lastStartManagedLocal = managesLocalProcess

        guard managesLocalProcess else {
            self.lifecycleGeneration &+= 1
            let generation = self.lifecycleGeneration
            self.stopping = false
            self.transition(to: .starting)
            do {
                _ = try await DaemonClient.shared.health()
                guard generation == self.lifecycleGeneration, !self.stopping else {
                    throw BackendServiceError.stoppedDuringHealthCheck
                }
                self.transition(to: .ready)
            } catch {
                if generation == self.lifecycleGeneration { self.transition(to: .stopped) }
                throw error
            }
            return
        }
        guard self.state != .starting, self.state != .ready else { return }
        self.lifecycleGeneration &+= 1
        self.restartTask?.cancel()
        self.restartTask = nil
        self.stopping = false
        let generation = self.lifecycleGeneration
        self.transition(to: .starting)
        do {
            await self.cleanStalePID()
            guard generation == self.lifecycleGeneration, !self.stopping else {
                throw BackendServiceError.stoppedDuringHealthCheck
            }
            try self.createDataDirIfNeeded()
            try await self.spawnAndWaitHealthy()
        } catch {
            if generation == self.lifecycleGeneration { await self.stop() }
            throw error
        }
    }

    func stop() async {
        self.lifecycleGeneration &+= 1
        self.restartTask?.cancel()
        self.restartTask = nil
        self.stopping = true
        self.readySince = nil
        await self.settleOwnedProcess()
    }

    private func settleOwnedProcess() async {
        if let stopTask {
            await stopTask.value
            return
        }
        let operation = Task {
            await self.stopOwnedProcess()
            self.stopTask = nil
        }
        self.stopTask = operation
        await operation.value
    }

    private func stopOwnedProcess() async {
        guard let proc = self.process else {
            self.transition(to: .stopped)
            return
        }
        guard proc.isRunning else {
            self.cleanup()
            return
        }

        logger.info("Stopping bobe backend (PID: \(proc.processIdentifier))")
        proc.terminate()

        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: terminateGrace)
        while proc.isRunning, clock.now < deadline {
            try? await clock.sleep(for: .milliseconds(100))
        }

        if proc.isRunning {
            logger.warning("bobe backend didn't exit gracefully, sending SIGKILL")
            kill(proc.processIdentifier, SIGKILL)
            let killDeadline = clock.now.advanced(by: .seconds(5))
            while proc.isRunning, clock.now < killDeadline {
                try? await clock.sleep(for: .milliseconds(100))
            }
        }
        guard !proc.isRunning else {
            self.lastError = "The managed daemon did not stop. No replacement was started."
            logger.error("Managed daemon has not exited; retaining process ownership")
            self.transition(to: .fatal)
            return
        }
        self.cleanup()
        logger.info("bobe backend stopped")
    }

    func userRestart() async throws {
        guard DaemonConfig.endpoint.canRestartRuntime else {
            throw BackendServiceError.restartUnavailable
        }
        self.restartCount = 0
        await self.stop()
        try await self.start()
    }

    private struct LocalEnrollmentRequest: Encodable {
        let bootstrapSecret: String
        let endpointId: String
        let label: String
        let protocolMin: Int
        let protocolMax: Int
        let capabilities: [String: Bool]

        enum CodingKeys: String, CodingKey {
            case bootstrapSecret = "bootstrap_secret"
            case endpointId = "endpoint_id"
            case label
            case protocolMin = "protocol_min"
            case protocolMax = "protocol_max"
            case capabilities
        }
    }

    private struct LocalEnrollmentResponse: Decodable {
        let endpointId: String
        let credential: String
        let runtimeAuthorityId: UUID
        let runtimeAuthorityEpoch: Int

        enum CodingKeys: String, CodingKey {
            case endpointId = "endpoint_id"
            case credential
            case runtimeAuthorityId = "runtime_authority_id"
            case runtimeAuthorityEpoch = "runtime_authority_epoch"
        }
    }

    private func enrollManagedLocal(bootstrapSecret: Data) async throws {
        let selection = EndpointStore.shared.selection
        guard let profile = EndpointStore.shared.profile,
              profile.mode == .managedLocal
        else { throw BackendServiceError.spawnFailed("Managed endpoint profile is unavailable") }
        var request = URLRequest(
            url: profile.baseURL.appendingPathComponent("v2/local-enrollment/complete")
        )
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try Self.localEnrollmentBody(
            bootstrapSecret: bootstrapSecret,
            endpointId: SurfaceIdentity.current()
        )
        let session = DaemonTransportPolicy.session(
            for: .managedLocal(bearerToken: nil),
            purpose: .attach
        )
        var enrollmentResponse: (Data, URLResponse)?
        var delay = healthCheckInitialDelay
        var lastConnectionError: (any Error)?
        for attempt in 1 ... healthCheckMaxAttempts {
            if self.stopping || EndpointStore.shared.selection != selection {
                throw BackendServiceError.stoppedDuringHealthCheck
            }
            if self.process?.isRunning != true { throw BackendServiceError.processExitedDuringHealthCheck }

            do {
                enrollmentResponse = try await session.data(for: request)
                break
            } catch {
                lastConnectionError = error
                guard attempt < healthCheckMaxAttempts else { break }
                logger.debug(
                    "Enrollment connection attempt \(attempt)/\(healthCheckMaxAttempts) failed, retrying in \(delay)s"
                )
                try await Task.sleep(for: .seconds(delay))
                delay = min(delay * healthCheckBackoffMultiplier, healthCheckMaxDelay)
            }
        }
        guard var (data, response) = enrollmentResponse else {
            throw lastConnectionError ?? BackendServiceError.healthCheckFailed
        }
        if (response as? HTTPURLResponse)?.statusCode == 409 {
            request.httpBody = try Self.localEnrollmentBody(
                bootstrapSecret: bootstrapSecret,
                endpointId: SurfaceIdentity.renew()
            )
            (data, response) = try await session.data(for: request)
        }
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            throw BackendServiceError.spawnFailed("Managed endpoint enrollment failed")
        }
        guard !self.stopping, EndpointStore.shared.selection == selection else {
            throw BackendServiceError.stoppedDuringHealthCheck
        }
        let result = try JSONDecoder().decode(LocalEnrollmentResponse.self, from: data)
        let reference = profile.credentialReference ?? UUID().uuidString
        let updated = EndpointProfile(
            id: profile.id,
            mode: profile.mode,
            baseURL: profile.baseURL,
            runtimeAuthorityId: result.runtimeAuthorityId,
            runtimeAuthorityEpoch: result.runtimeAuthorityEpoch,
            endpointId: UUID(uuidString: result.endpointId),
            friendlyName: profile.friendlyName,
            credentialReference: reference,
            trustMode: .loopback
        )
        try EndpointStore.shared.persist(updated, credential: result.credential)
    }

    private static func localEnrollmentBody(bootstrapSecret: Data, endpointId: UUID) throws -> Data {
        try JSONEncoder().encode(
            LocalEnrollmentRequest(
                bootstrapSecret: bootstrapSecret.base64EncodedString(),
                endpointId: endpointId.uuidString,
                label: Host.current().localizedName ?? "This Mac",
                protocolMin: Int(AttachWire.protocolMin),
                protocolMax: Int(AttachWire.protocolMax),
                capabilities: [
                    "microphone": true,
                    "playback": true,
                    "local_stt": true,
                    "local_tts": true,
                    "screen_context": true,
                ]
            )
        )
    }

    private func spawnAndWaitHealthy() async throws {
        guard self.process?.isRunning != true else {
            throw BackendServiceError.spawnFailed("The previous managed daemon is still running")
        }
        let generation = self.lifecycleGeneration
        let binaryPath = self.findBinaryPath()
        guard let binaryPath else {
            if ProcessInfo.processInfo.environment["BOBE_DEV"] != nil {
                logger.info("bobe binary not found — dev mode, expecting manual backend")
                self.transition(to: .ready)
                return
            }
            self.transition(to: .fatal)
            self.lastError = "Backend binary not found in app bundle. Try reinstalling BoBe."
            throw BackendServiceError.spawnFailed("bobe-daemon binary not found")
        }

        logger.info("Starting bobe backend: \(binaryPath, privacy: .public)")
        self.lastError = nil
        self.startupWarning = nil

        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: binaryPath)
        proc.arguments = [
            "serve",
            "--port", String(DaemonConfig.managedPort),
            "--managed-bootstrap-stdin",
        ]
        proc.currentDirectoryURL = self.dataDir

        var env = ProcessInfo.processInfo.environment
        env["HOME"] = FileManager.default.homeDirectoryForCurrentUser.path
        env["BOBE_DATA_DIR"] = self.dataDir.path
        guard let agentRuntime = Self.resolveAgentRuntime(
            environment: env,
            bundleURL: Bundle.main.bundleURL,
            isExecutableFile: { FileManager.default.isExecutableFile(atPath: $0) },
            isReadableFile: { FileManager.default.isReadableFile(atPath: $0) }
        ) else {
            self.transition(to: .fatal)
            self.lastError = "The bundled agent runtime is missing. Try reinstalling BoBe."
            throw BackendServiceError.spawnFailed("Node or BoBe agent files not found")
        }
        env["BOBE_NODE_PATH"] = agentRuntime.node
        env["BOBE_AGENT_ENTRY"] = agentRuntime.entry
        proc.environment = env

        let bootstrapPipe = Pipe()
        var bootstrapSecret = Data(count: 32)
        defer { bootstrapSecret.resetBytes(in: bootstrapSecret.startIndex..<bootstrapSecret.endIndex) }
        let status = bootstrapSecret.withUnsafeMutableBytes { bytes in
            guard let baseAddress = bytes.baseAddress else { return errSecParam }
            return SecRandomCopyBytes(kSecRandomDefault, bytes.count, baseAddress)
        }
        guard status == errSecSuccess else {
            throw BackendServiceError.spawnFailed("Could not create local enrollment secret")
        }
        proc.standardInput = bootstrapPipe
        let outPipe = Pipe()
        let errPipe = Pipe()
        proc.standardOutput = outPipe
        proc.standardError = errPipe

        let stderrBuf = StderrBuffer()
        outPipe.fileHandleForReading.readabilityHandler = { handle in
            if let line = String(data: handle.availableData, encoding: .utf8), !line.isEmpty {
                logger.info("[bobe-service] \(line.trimmingCharacters(in: .newlines), privacy: .public)")
            }
        }
        errPipe.fileHandleForReading.readabilityHandler = { handle in
            let data = handle.availableData
            if let line = String(data: data, encoding: .utf8), !line.isEmpty {
                let trimmed = line.trimmingCharacters(in: .newlines)
                logger.error("[bobe-service] \(trimmed, privacy: .public)")
                stderrBuf.append(trimmed)
            }
        }

        if self.isPortInUse(DaemonConfig.managedPort) {
            self.lastError =
                "Port \(DaemonConfig.managedPort) is already in use by another application. "
                    + "Close the conflicting application, then relaunch BoBe."
            throw BackendServiceError.healthCheckFailed
        }

        proc.terminationHandler = { [weak self] terminatedProc in
            Task { [weak self] in
                await self?.handleExit(
                    pid: terminatedProc.processIdentifier,
                    exitCode: Int(terminatedProc.terminationStatus)
                )
            }
        }
        do {
            try proc.run()
            self.process = proc
            self.writePID(proc.processIdentifier)
            try bootstrapPipe.fileHandleForWriting.write(contentsOf: bootstrapSecret)
            try bootstrapPipe.fileHandleForWriting.close()
        } catch {
            self.transition(to: .fatal)
            throw BackendServiceError.spawnFailed(error.localizedDescription)
        }

        do {
            let existingCredential = EndpointStore.shared.profile.flatMap(
                EndpointStore.shared.credential
            )
            if let profile = EndpointStore.shared.profile,
               profile.mode == .managedLocal,
               let existingCredential,
               (try? await EndpointAttachService.shared.attach(
                   profile: profile,
                   credential: existingCredential
               )) != nil {
                bootstrapSecret.resetBytes(in: bootstrapSecret.startIndex..<bootstrapSecret.endIndex)
            } else {
                try await self.enrollManagedLocal(bootstrapSecret: bootstrapSecret)
                bootstrapSecret.resetBytes(in: bootstrapSecret.startIndex..<bootstrapSecret.endIndex)
            }
            try await self.waitForHealth()
            guard generation == self.lifecycleGeneration, self.process === proc, !self.stopping else {
                throw BackendServiceError.stoppedDuringHealthCheck
            }
        } catch {
            guard generation == self.lifecycleGeneration else { throw error }
            let captured = stderrBuf.text
            if !captured.isEmpty {
                self.lastError = captured
                logger.error("Backend stderr on failure: \(captured, privacy: .public)")
            }
            throw error
        }
        self.transition(to: .ready)
        self.readySince = ContinuousClock().now
        logger.info("bobe backend healthy (PID: \(proc.processIdentifier))")
    }

    private func waitForHealth() async throws {
        var delay = healthCheckInitialDelay

        for attempt in 1 ... healthCheckMaxAttempts {
            if self.stopping { throw BackendServiceError.stoppedDuringHealthCheck }
            if self.process?.isRunning != true { throw BackendServiceError.processExitedDuringHealthCheck }

            do {
                let response = try await DaemonClient.shared.health()
                if response.services.database != "ok" {
                    logger.warning("Backend database degraded: \(response.services.database, privacy: .public)")
                    self.startupWarning = L10n.tr("app.service_warning.database_degraded")
                }
                return
            } catch {
                logger.debug("Health check attempt \(attempt)/\(healthCheckMaxAttempts) failed, retrying in \(delay)s")
                try await Task.sleep(for: .seconds(delay))
                delay = min(delay * healthCheckBackoffMultiplier, healthCheckMaxDelay)
            }
        }
        throw BackendServiceError.healthCheckFailed
    }

    private func handleExit(pid: Int32, exitCode: Int) {
        guard self.process?.processIdentifier == pid else {
            logger.debug("Ignoring stale termination callback for PID \(pid)")
            return
        }
        self.process = nil
        try? FileManager.default.removeItem(at: self.pidFilePath)

        guard !self.stopping else { return }

        logger.warning("bobe backend exited unexpectedly (code: \(exitCode))")
        if let readySince, readySince.duration(to: ContinuousClock().now) >= .seconds(60) {
            self.restartCount = 0
        }
        self.readySince = nil
        if self.state == .starting {
            self.transition(to: .stopped)
            return
        }
        self.transition(to: .crashed)

        self.scheduleAutomaticRestart()
    }

    private func scheduleAutomaticRestart() {
        self.restartCount += 1
        if self.restartCount > self.maxRestartAttempts {
            logger.error("bobe backend failed \(self.maxRestartAttempts) times, giving up")
            self.transition(to: .fatal)
            return
        }

        let backoffSeconds = self.restartCount
        logger.info("Restarting bobe backend in \(backoffSeconds)s (attempt \(self.restartCount)/\(self.maxRestartAttempts))")

        let generation = self.lifecycleGeneration
        self.restartTask?.cancel()
        self.restartTask = Task { [weak self] in
            do {
                try await Task.sleep(for: .seconds(backoffSeconds))
            } catch {
                return
            }
            guard let self else { return }
            await self.runAutomaticRestart(generation: generation)
        }
    }

    private func runAutomaticRestart(generation: UInt64) async {
        guard generation == self.lifecycleGeneration, !self.stopping else { return }
        do {
            self.transition(to: .starting)
            try await self.spawnAndWaitHealthy()
            self.restartTask = nil
        } catch {
            logger.error("Restart failed: \(error.localizedDescription, privacy: .public)")
            guard generation == self.lifecycleGeneration else { return }
            await self.settleOwnedProcess()
            guard generation == self.lifecycleGeneration, !self.stopping, self.process == nil else { return }
            self.transition(to: .crashed)
            self.scheduleAutomaticRestart()
        }
    }

    private func writePID(_ pid: Int32) {
        do { try "\(pid)".write(to: self.pidFilePath, atomically: true, encoding: .utf8) }
        catch { logger.error("Could not save managed daemon PID: \(error.localizedDescription, privacy: .public)") }
    }

    private func cleanStalePID() async {
        guard let pidStr = try? String(contentsOf: pidFilePath, encoding: .utf8),
              let pid = Int32(pidStr.trimmingCharacters(in: .whitespacesAndNewlines))
        else {
            if self.isPortInUse(DaemonConfig.managedPort) {
                logger.warning("Port \(DaemonConfig.managedPort) is in use but no PID file exists — another process may be bound")
            }
            return
        }

        switch self.inspectManagedProcess(pid: pid) {
        case let .verifiedDaemon(path):
            logger.info("Cleaning stale bobe daemon (PID: \(pid), path: \(path, privacy: .public))")
            kill(pid, SIGTERM)
            let clock = ContinuousClock()
            let deadline = clock.now.advanced(by: terminateGrace)
            while kill(pid, 0) == 0, clock.now < deadline {
                try? await clock.sleep(for: .milliseconds(100))
            }
            if case .verifiedDaemon = self.inspectManagedProcess(pid: pid) {
                kill(pid, SIGKILL)
            }
        case let .otherProcess(path):
            logger.warning(
                "PID file points to a different live process (PID: \(pid), path: \(path, privacy: .public)); refusing to terminate it"
            )
            if self.isPortInUse(DaemonConfig.managedPort) {
                logger.warning("Port \(DaemonConfig.managedPort) remains in use by another process")
            }
        case .notRunning:
            if self.isPortInUse(DaemonConfig.managedPort) {
                logger.warning("Port \(DaemonConfig.managedPort) in use but PID \(pid) from PID file is not running — stale PID file")
            }
        case .unverifiable:
            logger.warning("Could not verify PID \(pid) from PID file as bobe-daemon; refusing to terminate it")
            if self.isPortInUse(DaemonConfig.managedPort) {
                logger.warning("Port \(DaemonConfig.managedPort) remains in use by another process")
            }
        }
        try? FileManager.default.removeItem(at: self.pidFilePath)
    }

    private func inspectManagedProcess(pid: Int32) -> ManagedProcessIdentity {
        guard kill(pid, 0) == 0 else { return .notRunning }
        guard let path = self.processPath(for: pid) else { return .unverifiable }
        return self.isManagedDaemonPath(path) ? .verifiedDaemon(path: path) : .otherProcess(path: path)
    }

    private func processPath(for pid: Int32) -> String? {
        var buffer = [CChar](repeating: 0, count: Int(MAXPATHLEN))
        let written = buffer.withUnsafeMutableBytes { rawBuffer in
            proc_pidpath(pid, rawBuffer.baseAddress, UInt32(rawBuffer.count))
        }
        guard written > 0 else { return nil }
        let bytes = buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }
        return String(bytes: bytes, encoding: .utf8)
    }

    private func isManagedDaemonPath(_ path: String) -> Bool {
        guard let standardized = self.canonicalizedPath(path),
              let binaryPath = self.findBinaryPath(),
              let managedPath = self.canonicalizedPath(binaryPath)
        else {
            return false
        }

        return standardized == managedPath
    }

    private func canonicalizedPath(_ path: String) -> String? {
        URL(fileURLWithPath: path)
            .standardizedFileURL
            .resolvingSymlinksInPath()
            .path
    }

    private func isPortInUse(_ port: Int) -> Bool {
        let fd = socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { return false }
        defer { close(fd) }

        var addr = sockaddr_in()
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = UInt16(port).bigEndian
        addr.sin_addr.s_addr = inet_addr("127.0.0.1")

        let result = withUnsafePointer(to: &addr) { ptr in
            ptr.withMemoryRebound(to: sockaddr.self, capacity: 1) { sockPtr in
                connect(fd, sockPtr, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        return result == 0
    }

    private func cleanup() {
        self.process = nil
        self.transition(to: .stopped)
        try? FileManager.default.removeItem(at: self.pidFilePath)
    }

    private func createDataDirIfNeeded() throws {
        try FileManager.default.createDirectory(at: self.dataDir, withIntermediateDirectories: true)
    }

    struct AgentRuntimePaths: Equatable, Sendable {
        let node: String
        let entry: String
    }

    nonisolated static func resolveAgentRuntime(
        environment: [String: String],
        bundleURL: URL,
        isExecutableFile: (String) -> Bool,
        isReadableFile: (String) -> Bool
    ) -> AgentRuntimePaths? {
        let bundled = AgentRuntimePaths(
            node: bundleURL.appendingPathComponent("Contents/MacOS/node").path,
            entry: bundleURL.appendingPathComponent("Contents/Resources/agent/dist/main.js").path
        )
        if isExecutableFile(bundled.node), isReadableFile(bundled.entry) {
            return bundled
        }

        if let node = environment["BOBE_NODE_PATH"],
           let entry = environment["BOBE_AGENT_ENTRY"],
           node.hasPrefix("/"), entry.hasPrefix("/"),
           isExecutableFile(node), isReadableFile(entry) {
            return AgentRuntimePaths(node: node, entry: entry)
        }
        return nil
    }

    private func findBinaryPath() -> String? {
        if let execURL = Bundle.main.executableURL {
            let siblingPath = execURL.deletingLastPathComponent()
                .appendingPathComponent("bobe-daemon").path
            if FileManager.default.isExecutableFile(atPath: siblingPath) {
                return siblingPath
            }
        }

        let devPaths = [
            FileManager.default.homeDirectoryForCurrentUser
                .appendingPathComponent(".cargo/bin/bobe").path,
            "/usr/local/bin/bobe",
        ]
        for path in devPaths where FileManager.default.isExecutableFile(atPath: path) {
            return path
        }
        return nil
    }
}

enum BackendServiceError: Error, LocalizedError {
    case healthCheckFailed
    case spawnFailed(String)
    case stoppedDuringHealthCheck
    case processExitedDuringHealthCheck
    case restartUnavailable

    var errorDescription: String? {
        switch self {
        case .healthCheckFailed: "Backend health check failed after maximum attempts"
        case let .spawnFailed(msg): "Failed to start backend: \(msg)"
        case .stoppedDuringHealthCheck: "Service was stopped during health check"
        case .processExitedDuringHealthCheck: "Backend process exited during health check"
        case .restartUnavailable: "This BoBe runtime is managed outside the app"
        }
    }
}

private final class StderrBuffer: Sendable {
    private let lines = OSAllocatedUnfairLock(initialState: [String]())

    func append(_ line: String) {
        self.lines.withLock { lines in
            lines.append(line)
            if lines.count > stderrBufferMaxLines {
                lines.removeFirst(lines.count - stderrBufferMaxLines)
            }
        }
    }

    var text: String {
        self.lines.withLock { $0.joined(separator: "\n") }
    }
}
