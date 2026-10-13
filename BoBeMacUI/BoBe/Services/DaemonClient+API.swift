import Foundation

extension DaemonClient {
    func agentRuns() async throws -> [AgentRunWire] {
        try await self.fetch("/runs")
    }

    func stopAgentRun(_ id: UUID) async throws {
        try await self.fetchVoid(pathSegments: ["runs", id.uuidString, "cancel"], method: "POST")
    }

    func guideAgentRun(_ id: UUID, text: String, followUp: Bool, inputId: UUID) async throws {
        struct Input: Encodable {
            let text: String
            let followUp: Bool
            enum CodingKeys: String, CodingKey { case text, followUp = "follow_up" }
        }
        try await self.fetchVoid(pathSegments: ["runs", id.uuidString, "inputs", inputId.uuidString],
                                 method: "PUT", body: Input(text: text, followUp: followUp))
    }

    func devices() async throws -> [DeviceWire] {
        try await self.fetch("/endpoints")
    }

    func removeDevice(_ id: String) async throws {
        try await self.fetchVoid(pathSegments: ["endpoints", id], method: "DELETE")
    }

    func createPhoneInvitation(label: String) async throws -> PhoneInvitationWire {
        struct Request: Encodable {
            let kind: String
            let label: String
        }
        return try await self.fetch("/pairing/invitations", method: "POST", body: Request(kind: "phone", label: label))
    }

    func setServerReach(host: String, allowedHosts: [String]) async throws {
        struct Changes: Encodable {
            let host: String
            let allowedHosts: [String]
            enum CodingKeys: String, CodingKey {
                case host = "server.host"
                case allowedHosts = "server.allowed_hosts"
            }
        }
        struct Request: Encodable {
            let changes: Changes
        }
        try await self.fetchVoid(
            pathSegments: ["config"],
            method: "PATCH",
            body: Request(changes: Changes(host: host, allowedHosts: allowedHosts))
        )
    }

    func ownerQuestions() async throws -> [OwnerInputWire] {
        try await self.fetch("/owner-input")
    }

    func markOwnerQuestionPresented(_ id: UUID) async throws {
        try await self.fetchVoid(pathSegments: ["owner-input", id.uuidString, "presented"], method: "POST")
    }

    func answerOwnerQuestion(_ id: UUID, response: String, responseId: UUID) async throws -> OwnerInputWire {
        struct Answer: Encodable {
            let response: String
            let responseId: UUID
            enum CodingKeys: String, CodingKey { case response, responseId = "response_id" }
        }
        return try await self.fetch(pathSegments: ["owner-input", id.uuidString, "answer"], method: "POST",
                                    body: Answer(response: response, responseId: responseId))
    }

    func markResponsePresented(_ id: UUID) async throws {
        try await self.fetchVoid(pathSegments: ["conversation", id.uuidString, "presented"], method: "POST")
    }

    func agentSchedules() async throws -> [AgentScheduleWire] {
        try await self.fetch("/agent/schedules")
    }

    func setScheduleEnabled(_ id: UUID, enabled: Bool) async throws -> AgentScheduleWire {
        struct Enabled: Encodable { let enabled: Bool }
        return try await self.fetch(pathSegments: ["agent", "schedules", id.uuidString, "enabled"],
                                    method: "PUT", body: Enabled(enabled: enabled))
    }

    func cancelSchedule(_ id: UUID) async throws {
        try await self.fetchVoid(pathSegments: ["agent", "schedules", id.uuidString], method: "DELETE")
    }

    func createSchedule(title: String, task: String, at: Date, everySeconds: Int?, requestId: UUID) async throws -> AgentScheduleWire {
        struct Schedule: Encodable {
            let title: String
            let task: String
            let at: String
            let everySeconds: Int?
            enum CodingKeys: String, CodingKey { case title, task, at, everySeconds = "every_seconds" }
        }
        struct Request: Encodable {
            let requestId: UUID
            let schedule: Schedule
            enum CodingKeys: String, CodingKey { case schedule, requestId = "request_id" }
        }
        return try await self.fetch("/agent/schedules", method: "POST", body: Request(requestId: requestId,
                                                                                      schedule: Schedule(title: title, task: task, at: at.ISO8601Format(), everySeconds: everySeconds)))
    }

    func agentTools() async throws -> AgentToolSelection {
        try await self.fetch("/agent/tools")
    }

    func setAgentTools(_ enabled: [AgentToolWire]) async throws -> AgentToolSelection {
        struct Selection: Encodable {
            let enabledTools: [AgentToolWire]
            enum CodingKeys: String, CodingKey { case enabledTools = "enabled_tools" }
        }
        return try await self.fetch("/agent/tools", method: "PUT", body: Selection(enabledTools: enabled))
    }

    func backgroundJobs() async throws -> [AgentJobWire] {
        try await self.fetch("/agent/jobs")
    }

    func cancelBackgroundJob(_ id: UUID) async throws -> AgentJobWire {
        try await self.fetch(pathSegments: ["agent", "jobs", id.uuidString, "cancel"], method: "POST")
    }

    func resumeBackgroundJob(_ id: UUID) async throws -> AgentJobWire {
        try await self.fetch(pathSegments: ["agent", "jobs", id.uuidString, "resume"], method: "POST")
    }

    func getMCPConfig() async throws -> MCPConfigDocumentResponse {
        try await fetch("/tools/mcp/config")
    }

    func validateMCPConfig(_ request: MCPConfigMutationRequest) async throws -> MCPConfigValidateResponse {
        try await fetch("/tools/mcp/config/validate", method: "POST", body: request)
    }

    func saveMCPConfig(_ request: MCPConfigMutationRequest) async throws -> MCPConfigSaveResponse {
        try await fetch("/tools/mcp/config", method: "PUT", body: request)
    }

    func resetMCPConfig() async throws -> MCPConfigResetResponse {
        try await fetch("/tools/mcp/config", method: "DELETE")
    }

    struct PrivacyPurgeResponse: Decodable {
        let message: String
        let deleted: Deleted
        let retained: [String]
        let reloadWarning: String?

        enum CodingKeys: String, CodingKey {
            case message
            case deleted
            case retained
            case reloadWarning = "reload_warning"
        }

        struct Deleted: Decodable {
            let learnedSkills: Int
            let primarySkillReset: Bool
            let workspaceFiles: Int

            enum CodingKeys: String, CodingKey {
                case learnedSkills = "learned_skills"
                case primarySkillReset = "primary_skill_reset"
                case workspaceFiles = "workspace_files"
            }
        }

        var resultSummary: String {
            let deletion = "Deleted \(self.deleted.workspaceFiles) workspace file(s) and \(self.deleted.learnedSkills) learned skill(s)."
            let retained = "Retained: \(self.retained.joined(separator: ", "))."
            let warning = self.reloadWarning.map { "\nWarning: \($0)" } ?? ""
            return "\(self.message)\n\(deletion)\n\(retained)\(warning)"
        }
    }

    func purgePersonalData() async throws -> PrivacyPurgeResponse {
        try await fetch(
            "/privacy/data",
            method: "DELETE",
            requestTimeout: PrivacyWire.purgeRequestTimeoutSeconds
        )
    }

    func getSettings() async throws -> DaemonSettings {
        try await fetch("/settings")
    }

    func updateSettings(_ request: SettingsUpdateRequest) async throws -> SettingsUpdateResponse {
        try await fetch("/settings", method: "PATCH", body: request)
    }

    func listProviders() async throws -> ProvidersResponse {
        try await fetch("/providers")
    }

    func upsertProvider(
        _ id: String,
        request: ProviderUpsertRequest
    ) async throws -> ProviderMutationResponse {
        try await fetch(pathSegments: ["providers", id], method: "PUT", body: request)
    }

    func listModels(profile: String) async throws -> ProviderModelsResponse {
        try await fetch("/models", queryItems: [URLQueryItem(name: "profile", value: profile)])
    }

    func setProviderCredential(_ id: String, apiKey: String) async throws -> ProviderMutationResponse {
        struct Credential: Encodable {
            let apiKey: String
            enum CodingKeys: String, CodingKey { case apiKey = "api_key" }
        }
        return try await fetch(
            pathSegments: ["providers", id, "credential"],
            method: "PUT",
            body: Credential(apiKey: apiKey)
        )
    }

    func activateProvider(
        _ id: String,
        request: ProviderActivationRequest
    ) async throws -> ProviderMutationResponse {
        try await fetch(
            pathSegments: ["providers", id, "activation"],
            method: "PUT",
            body: request
        )
    }

    func setProviderRoute(_ workload: ProviderWorkload, route: WorkloadRouteWire) async throws -> ProviderMutationResponse {
        try await fetch("/provider-routes", method: "PATCH", body: [workload.rawValue: route])
    }

    func removeProvider(_ id: String) async throws -> ProviderMutationResponse {
        try await fetch(pathSegments: ["providers", id], method: "DELETE")
    }

    func clearProviderCredential(_ id: String) async throws -> ProviderMutationResponse {
        try await fetch(pathSegments: ["providers", id, "credential"], method: "DELETE")
    }

    func logoutProvider(_ id: String) async throws {
        try await fetchVoid(pathSegments: ["auth", id, "logout"], method: "POST")
    }

    func testProvider(_ route: WorkloadRouteWire) async throws -> ProviderProbeResponse {
        try await fetch(
            pathSegments: ["providers", route.profile, "probe"], method: "POST",
            body: ProviderProbeRequest(model: route.model ?? "", reasoningEffort: route.reasoningEffort),
            requestTimeout: 90
        )
    }

    func getStatus() async throws -> StatusResponse {
        try await fetch("/status")
    }

    func getCurrentConversation() async throws -> ConversationSnapshotResponse {
        try await fetch("/conversation/current")
    }

    func getAuthStatus(profile: String) async throws -> AuthStatusResponse {
        try await fetch(pathSegments: ["auth", profile, "status"])
    }

    func startLocalRuntimeInstall(_ request: LocalRuntimeInstallRequest) async throws {
        try await fetchVoid("/local-runtime/install", method: "POST", body: request)
    }

    func cancelLocalRuntimeInstall() async throws {
        try await fetchVoid("/local-runtime/cancel", method: "POST")
    }

    func streamLocalRuntimeStatus(
        onSnapshot: @Sendable @escaping (LocalRuntimeSnapshot) -> Void
    ) async throws {
        var reconnectAttempt = 0
        while !Task.isCancelled {
            let url = self.endpointURL("local-runtime/status")
            var request = URLRequest(url: url)
            self.endpoint.authorize(&request)
            request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
            request.timeoutInterval = 0

            do {
                let (bytes, response) = try await self.session(for: .finiteEventStream)
                    .bytes(for: request)
                guard let httpResponse = response as? HTTPURLResponse,
                      httpResponse.statusCode == 200
                else {
                    throw DaemonError.invalidResponse
                }

                reconnectAttempt = 0
                var receivedSnapshot = false
                for try await line in bytes.lines {
                    if Task.isCancelled {
                        return
                    }
                    guard line.hasPrefix("data: ") else { continue }
                    let jsonStr = String(line.dropFirst(6))
                    guard let data = jsonStr.data(using: .utf8) else { continue }
                    let snapshot = try JSONDecoder().decode(LocalRuntimeSnapshot.self, from: data)
                    receivedSnapshot = true
                    onSnapshot(snapshot)
                    if [.complete, .canceled, .failed].contains(snapshot.status) {
                        return
                    }
                }
                if receivedSnapshot {
                    reconnectAttempt = 0
                }
            } catch is CancellationError {
                return
            } catch {
                reconnectAttempt += 1
                if reconnectAttempt >= 10 {
                    throw error
                }
            }

            if reconnectAttempt == 0 {
                reconnectAttempt = 1
            }
            let delay = min(pow(2, Double(max(0, reconnectAttempt - 1))), 15)
            try await Task.sleep(for: .seconds(delay))
        }
    }

    func voiceInstallStatus() async throws -> VoiceInstallSnapshot {
        try await self.fetch("/voice/install/status")
    }

    func startVoiceInstall(kind: String) async throws {
        try await self.fetchVoid(
            pathSegments: ["voice", "install", "start", kind],
            method: "POST"
        )
    }

    func cancelVoiceInstall() async throws {
        try await self.fetchVoid("/voice/install/cancel", method: "POST")
    }

    func startProviderLogin(profile: String) async throws {
        try await self.fetchVoid(pathSegments: ["auth", profile, "login", "start"], method: "POST")
    }

    func cancelProviderLogin(profile: String) async throws {
        try await self.fetchVoid(pathSegments: ["auth", profile, "login", "cancel"], method: "POST")
    }

    func providerLoginStatus(profile: String) async throws -> ProviderLoginPhase {
        try await self.fetch(pathSegments: ["auth", profile, "login", "status"])
    }

    func submitProviderLogin(profile: String, value: String) async throws {
        struct Submission: Encodable { let value: String }
        try await self.fetchVoid(
            pathSegments: ["auth", profile, "login", "submit"],
            method: "POST",
            body: Submission(value: value)
        )
    }

    func streamProviderLogin(
        profile: String,
        onPhase: @Sendable @escaping (ProviderLoginPhase) async -> Void
    ) async throws {
        let url = self.endpointURL(pathSegments: ["auth", profile, "login", "events"])
        var request = URLRequest(url: url)
        self.endpoint.authorize(&request)
        request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
        request.timeoutInterval = 0

        let (bytes, response) = try await self.session(for: .finiteEventStream)
            .bytes(for: request)
        guard let httpResponse = response as? HTTPURLResponse,
              httpResponse.statusCode == 200
        else {
            throw DaemonError.invalidResponse
        }

        let decoder = JSONDecoder()
        for try await line in bytes.lines {
            if Task.isCancelled {
                break
            }
            guard line.hasPrefix("data: ") else { continue }
            let jsonStr = String(line.dropFirst(6))
            guard let data = jsonStr.data(using: .utf8) else { continue }
            let phase = try decoder.decode(ProviderLoginPhase.self, from: data)
            await onPhase(phase)
            if phase.isTerminal {
                break
            }
        }
    }
}
