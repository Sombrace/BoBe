@testable import BoBe
import Foundation
import Testing

@Suite("Lifecycle and transport policy")
struct LifecycleTransportPolicyTests {
    @Test
    func powerTransitionsIgnoreDuplicateNotifications() {
        var state = SurfacePowerState.running

        let firstSleep = state.prepareForSleep()
        #expect(firstSleep)
        #expect(state == .sleeping)
        let duplicateSleep = state.prepareForSleep()
        #expect(!duplicateSleep)
        let firstWake = state.resumeAfterWake()
        #expect(firstWake)
        #expect(state == .running)
        let duplicateWake = state.resumeAfterWake()
        #expect(!duplicateWake)
    }

    @Test
    func stoppedAndTerminatingStatesRejectPowerTransitions() {
        var stopped = SurfacePowerState.stopped
        let stoppedSleep = stopped.prepareForSleep()
        let stoppedWake = stopped.resumeAfterWake()
        #expect(!stoppedSleep)
        #expect(!stoppedWake)

        var terminating = SurfacePowerState.terminating
        let terminatingSleep = terminating.prepareForSleep()
        let terminatingWake = terminating.resumeAfterWake()
        #expect(!terminatingSleep)
        #expect(!terminatingWake)
    }

    @Test(arguments: [
        DaemonTransportPurpose.interactiveHTTP,
        .persistentEventStream,
        .finiteEventStream,
        .attach,
        .readinessProbe,
        .realtimeWebSocket,
        .bodyLoopbackWebSocket,
    ])
    func loopbackNeverWaitsForConnectivity(purpose: DaemonTransportPurpose) throws {
        let endpoint = DaemonEndpoint.attachedLocal(
            baseURL: try #require(URL(string: "http://127.0.0.1:8766")),
            bearerToken: nil
        )
        let configuration = DaemonTransportPolicy.configuration(for: endpoint, purpose: purpose)
        #expect(!configuration.waitsForConnectivity)
        #expect(configuration.allowsConstrainedNetworkAccess)
        #expect(configuration.allowsExpensiveNetworkAccess)
    }

    @Test(arguments: [
        (DaemonTransportPurpose.persistentEventStream, true),
        (.finiteEventStream, true),
        (.realtimeWebSocket, true),
        (.interactiveHTTP, false),
        (.attach, false),
        (.readinessProbe, false),
        (.bodyLoopbackWebSocket, false),
    ])
    func remoteLongLivedTransportsWaitForConnectivity(purpose: DaemonTransportPurpose, waits: Bool) throws {
        let endpoint = DaemonEndpoint.remote(
            baseURL: try #require(URL(string: "https://bobe.example.test")),
            bearerToken: "secret"
        )
        #expect(DaemonTransportPolicy.configuration(for: endpoint, purpose: purpose).waitsForConnectivity == waits)
    }

    @Test
    func finiteStreamsUseLongLivedResourcePolicy() throws {
        let endpoint = DaemonEndpoint.remote(
            baseURL: try #require(URL(string: "https://bobe.example.test")),
            bearerToken: "secret"
        )
        let finite = DaemonTransportPolicy.configuration(
            for: endpoint,
            purpose: .finiteEventStream
        )
        let interactive = DaemonTransportPolicy.configuration(
            for: endpoint,
            purpose: .interactiveHTTP
        )
        let readiness = DaemonTransportPolicy.configuration(
            for: endpoint,
            purpose: .readinessProbe
        )

        #expect(finite.timeoutIntervalForRequest == 60)
        #expect(finite.timeoutIntervalForResource == 7 * 24 * 60 * 60)
        #expect(interactive.timeoutIntervalForRequest == 10)
        #expect(interactive.timeoutIntervalForResource == 300)
        #expect(readiness.timeoutIntervalForRequest == 2)
    }
}
