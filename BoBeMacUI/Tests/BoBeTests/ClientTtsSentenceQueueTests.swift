@testable import BoBe
import Foundation
import Testing

@Suite("Client TTS sentence queue")
struct ClientTtsSentenceQueueTests {
    @Test
    func boundedSentenceQueuePreservesOrderUnderBackpressure() async throws {
        let queue = ClientTtsSentenceQueue(capacity: 1)
        let first = ClientTtsSentence(turnId: "turn", sequence: 0, text: "first")
        let second = ClientTtsSentence(turnId: "turn", sequence: 1, text: "second")

        #expect(await queue.send(first))
        let secondSend = Task { await queue.send(second) }
        await Task.yield()

        #expect(await queue.next()?.sequence == 0)
        #expect(await secondSend.value)
        await queue.finish()
        #expect(await queue.next()?.sequence == 1)
        #expect(await queue.next() == nil)
    }

    @Test
    func cancellingSentenceQueueRejectsBlockedProducer() async throws {
        let queue = ClientTtsSentenceQueue(capacity: 1)
        let first = ClientTtsSentence(turnId: "turn", sequence: 0, text: "first")
        let second = ClientTtsSentence(turnId: "turn", sequence: 1, text: "second")

        #expect(await queue.send(first))
        let secondSend = Task { await queue.send(second) }
        await Task.yield()
        await queue.cancel()

        #expect(!(await secondSend.value))
        #expect(await queue.next() == nil)
    }
}
