import Darwin
import Foundation
import SystemConfiguration

enum NetworkReach {
    static let anyAddress = "0.0.0.0"

    static func hosts() -> [String] {
        var hosts: [String] = []
        if let name = SCDynamicStoreCopyLocalHostName(nil) as String?, !name.isEmpty {
            hosts.append("\(name.lowercased()).local")
        }
        for address in self.ipv4Addresses() where !hosts.contains(address) {
            hosts.append(address)
        }
        return hosts
    }

    @MainActor
    static func set(_ reachable: Bool, endpoint: EndpointSelection) async throws {
        let client = DaemonClient(endpoint: endpoint.endpoint)
        if reachable {
            try await client.setServerReach(host: Self.anyAddress, allowedHosts: Self.hosts())
        } else {
            try await client.setServerReach(host: DaemonConfig.defaultHost, allowedHosts: [])
        }
        try await BackendService.shared.userRestart()
    }

    private static func ipv4Addresses() -> [String] {
        var list: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&list) == 0, let first = list else { return [] }
        defer { freeifaddrs(list) }
        var addresses: [String] = []
        var tunnels: [String] = []
        for interface in sequence(first: first, next: { $0.pointee.ifa_next }) {
            let flags = Int32(interface.pointee.ifa_flags)
            guard flags & IFF_UP != 0, flags & IFF_RUNNING != 0, flags & IFF_LOOPBACK == 0,
                  let address = interface.pointee.ifa_addr,
                  address.pointee.sa_family == sa_family_t(AF_INET)
            else { continue }
            let value = address.withMemoryRebound(to: sockaddr_in.self, capacity: 1) {
                UInt32(bigEndian: $0.pointee.sin_addr.s_addr)
            }
            guard value >> 16 != 0xA9FE else { continue }
            let text = "\(value >> 24).\(value >> 16 & 0xFF).\(value >> 8 & 0xFF).\(value & 0xFF)"
            if flags & IFF_POINTOPOINT != 0 {
                tunnels.append(text)
            } else {
                addresses.append(text)
            }
        }
        return addresses + tunnels
    }
}
