import Foundation

struct PairingInvite: Equatable, Sendable {
    static let defaultPort = 8766

    let origins: [URL]
    let pin: Data?
    let code: String

    init?(link: URL) {
        guard link.scheme?.lowercased() == "bobe",
              link.host()?.lowercased() == "pair",
              let items = URLComponents(url: link, resolvingAgainstBaseURL: false)?.queryItems,
              items.first(where: { $0.name == "v" })?.value == "1",
              let code = items.first(where: { $0.name == "c" })?.value,
              Self.isCode(code),
              let pinText = items.first(where: { $0.name == "p" })?.value,
              let pin = Data(base64URLEncoded: pinText),
              pin.count == 32
        else { return nil }
        let origins = items
            .filter { $0.name == "o" }
            .compactMap { $0.value.flatMap(Self.origin(hostPort:)) }
        guard !origins.isEmpty else { return nil }
        self.origins = origins
        self.pin = pin
        self.code = code
    }

    init?(address: String, code: String) {
        guard Self.isCode(code), let origin = Self.origin(typed: address) else { return nil }
        self.origins = [origin]
        self.pin = nil
        self.code = code
    }

    static func isCode(_ value: String) -> Bool {
        value.count == 6 && value.unicodeScalars.allSatisfy { ("0" ... "9").contains($0) }
    }

    static func origin(hostPort: String) -> URL? {
        guard let components = URLComponents(string: "https://" + hostPort),
              components.port != nil
        else { return nil }
        return self.canonical(components)
    }

    static func origin(typed: String) -> URL? {
        let trimmed = typed.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        let candidate = trimmed.contains("://") ? trimmed : "https://" + trimmed
        guard var components = URLComponents(string: candidate),
              components.scheme?.lowercased() == "https"
        else { return nil }
        if components.port == nil {
            components.port = self.defaultPort
        }
        return self.canonical(components)
    }

    private static func canonical(_ input: URLComponents) -> URL? {
        var components = input
        guard let host = components.host,
              !host.isEmpty,
              components.user == nil,
              components.password == nil,
              components.query == nil,
              components.fragment == nil,
              components.path.isEmpty || components.path == "/"
        else { return nil }
        components.scheme = "https"
        if !host.contains(":") {
            components.host = host.lowercased()
        }
        components.path = ""
        if components.port == 443 {
            components.port = nil
        }
        return components.url
    }
}
