import Foundation

enum AgentToolWire: String, Codable, Sendable, CaseIterable {
    case read
    case bash
    case edit
    case write
    case grep
    case find
    case ls
    case subagent
    case backgroundJobs = "background_jobs"
}

struct AgentToolState: Codable, Identifiable, Sendable {
    let name: AgentToolWire
    let description: String
    let enabled: Bool
    let active: Bool
    var id: AgentToolWire { self.name }
}

struct AgentToolSelection: Codable, Sendable {
    let tools: [AgentToolState]
    let restartRequired: Bool
    enum CodingKeys: String, CodingKey {
        case tools
        case restartRequired = "restart_required"
    }
}

enum AgentJobStatusWire: String, Decodable, Sendable {
    case queued
    case running
    case cancelRequested = "cancel_requested"
    case completed
    case failed
    case cancelled
    case interrupted
    case awaitingInput = "awaiting_input"
}

struct AgentJobWire: Decodable, Identifiable, Sendable {
    let id: UUID
    let title: String
    let task: String
    let status: AgentJobStatusWire
    let attempt: Int
    let output: String?
    let error: String?
    let deliveryStatus: String

    var canCancel: Bool { self.status == .queued || self.status == .running || self.status == .awaitingInput }
    var canResume: Bool {
        (self.status == .failed || self.status == .interrupted) && self.deliveryStatus != "delivering"
    }
    enum CodingKeys: String, CodingKey {
        case id, title, task, status, attempt, output, error
        case deliveryStatus = "delivery_status"
    }
}

struct AgentScheduleWire: Decodable, Identifiable, Sendable {
    let id: UUID
    let title: String
    let task: String
    let nextRunAt: String
    let everySeconds: Int?
    let enabled: Bool
    let generation: Int
    let lastError: String?

    var nextRunDate: Date? {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return formatter.date(from: self.nextRunAt)
    }

    enum CodingKeys: String, CodingKey {
        case id, title, task, enabled, generation
        case nextRunAt = "next_run_at", everySeconds = "every_seconds", lastError = "last_error"
    }
}

struct AgentRunWire: Decodable, Identifiable, Sendable {
    let id: UUID
    let kind: AgentRunKindWire
    let stopping: Bool
    let queued: Bool
}

enum AgentRunKindWire: String, Decodable, Sendable {
    case user
    case proactive
    case background
    case delivery
    case voice
}

enum OwnerInputKindWire: String, Decodable, Sendable {
    case approval
    case information
}

enum OwnerInputStatusWire: String, Decodable, Sendable {
    case pending
    case answered
    case declined
    case cancelled
    case expired
}

struct OwnerInputChoiceWire: Decodable, Identifiable, Sendable {
    let value: String
    let label: String
    var id: String { self.value }
}

struct OwnerInputWire: Decodable, Identifiable, Sendable {
    let id: UUID
    let kind: OwnerInputKindWire
    let question: String
    let action: String
    let prompt: String
    let choices: [OwnerInputChoiceWire]
    let status: OwnerInputStatusWire
    let expiresAt: String
    let response: String?
    let resumedJobId: UUID?

    enum CodingKeys: String, CodingKey {
        case id, kind, question, action, prompt, choices, status, response
        case expiresAt = "expires_at", resumedJobId = "resumed_job_id"
    }
}
