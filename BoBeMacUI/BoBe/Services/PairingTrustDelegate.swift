import CryptoKit
import Foundation
import PairingClient
import Security
import Synchronization

nonisolated final class PairingTrustDelegate: NSObject, URLSessionTaskDelegate, Sendable {
    private let observedSPKI = Mutex<Data?>(nil)
    private let expectedSPKI: Data?
    private let acceptsUntrustedPairingCertificate: Bool

    init(expectedSPKI: Data? = nil, acceptsUntrustedPairingCertificate: Bool = false) {
        self.expectedSPKI = expectedSPKI
        self.acceptsUntrustedPairingCertificate = acceptsUntrustedPairingCertificate
    }

    func observedPin() -> Data? {
        self.observedSPKI.withLock { $0 }
    }

    func urlSession(
        _ session: URLSession,
        didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping @Sendable (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        self.evaluate(challenge, completionHandler: completionHandler)
    }

    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        didReceive challenge: URLAuthenticationChallenge,
        completionHandler: @escaping @Sendable (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        self.evaluate(challenge, completionHandler: completionHandler)
    }

    private func evaluate(
        _ challenge: URLAuthenticationChallenge,
        completionHandler: @escaping @Sendable (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
    ) {
        guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
              let trust = challenge.protectionSpace.serverTrust,
              let certificate = SecTrustCopyCertificateChain(trust) as? [SecCertificate],
              let leaf = certificate.first
        else {
            completionHandler(.performDefaultHandling, nil)
            return
        }
        let der = SecCertificateCopyData(leaf) as Data
        guard let spki = PairingCertificate.spkiSHA256(certificateDER: der) else {
            completionHandler(.cancelAuthenticationChallenge, nil)
            return
        }
        if let expectedSPKI {
            guard spki == expectedSPKI else {
                completionHandler(.cancelAuthenticationChallenge, nil)
                return
            }
            completionHandler(.useCredential, URLCredential(trust: trust))
            return
        }
        guard self.acceptsUntrustedPairingCertificate else {
            completionHandler(.performDefaultHandling, nil)
            return
        }
        self.observedSPKI.withLock { $0 = spki }
        completionHandler(.useCredential, URLCredential(trust: trust))
    }
}
