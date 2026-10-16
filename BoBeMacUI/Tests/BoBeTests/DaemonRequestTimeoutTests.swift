@testable import BoBe
import Foundation
import Testing

@Suite("Daemon request timeouts")
struct DaemonRequestTimeoutTests {
    @Test
    func mcpChangesWaitForServerStartupWhileOtherCallsKeepTheirTimeout() async throws {
        let endpoint = DaemonEndpoint.attachedLocal(
            baseURL: try #require(URL(string: "http://127.0.0.1:9")),
            bearerToken: nil
        )
        let log = RequestLog()
        let client = DaemonClient(endpoint: endpoint, transport: { request in
            await log.record(request)
            return try Self.reply(to: request)
        })
        var change = SettingsUpdateRequest()
        change.mcpEnabled = true

        _ = try await client.updateSettings(change)
        _ = try await client.saveMCPConfig(MCPConfigMutationRequest(rawJson: "{}"))
        _ = try await client.resetMCPConfig()
        _ = try await client.validateMCPConfig(MCPConfigMutationRequest(rawJson: "{}"))
        _ = try await client.getMCPConfig()

        let fetchTimeout = await client.fetchTimeout
        #expect(await log.timeouts == [
            "PATCH /settings": DaemonClient.mcpStartupTimeout,
            "PUT /tools/mcp/config": DaemonClient.mcpStartupTimeout,
            "DELETE /tools/mcp/config": DaemonClient.mcpStartupTimeout,
            "POST /tools/mcp/config/validate": fetchTimeout,
            "GET /tools/mcp/config": fetchTimeout,
        ])
        #expect(DaemonClient.mcpStartupTimeout > fetchTimeout)
        let session = DaemonTransportPolicy.configuration(for: endpoint, purpose: .interactiveHTTP)
        #expect(session.timeoutIntervalForResource >= DaemonClient.mcpStartupTimeout)
    }

    private static func reply(to request: URLRequest) throws -> (Data, URLResponse) {
        let url = try #require(request.url)
        let body = switch (request.httpMethod ?? "GET", url.path) {
        case ("PATCH", "/settings"):
            #"{"message":"Updated 1 setting(s)","restart_required_fields":[]}"#
        case ("PUT", "/tools/mcp/config"):
            #"{"message":"Saved","raw_json":"{}","servers":[]}"#
        case ("DELETE", "/tools/mcp/config"):
            #"{"message":"Reset","raw_json":"{}"}"#
        case ("POST", "/tools/mcp/config/validate"):
            #"{"valid":true,"normalized_json":"{}","server_count":0,"errors":[]}"#
        default:
            #"{"raw_json":"{}","servers":[]}"#
        }
        let response = try #require(HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: nil))
        return (Data(body.utf8), response)
    }
}

private actor RequestLog {
    private(set) var timeouts: [String: TimeInterval] = [:]

    func record(_ request: URLRequest) {
        let route = "\(request.httpMethod ?? "GET") \(request.url?.path ?? "")"
        self.timeouts[route] = request.timeoutInterval
    }
}
