import Foundation

nonisolated enum SurfaceIdentity {
    static let defaultsKey = "bobe.surface_id"

    static func current(defaults: UserDefaults = .standard) -> UUID {
        if let raw = defaults.string(forKey: self.defaultsKey),
           let id = UUID(uuidString: raw) {
            return id
        }
        return self.renew(defaults: defaults)
    }

    static func renew(defaults: UserDefaults = .standard) -> UUID {
        let id = UUID()
        defaults.set(id.uuidString, forKey: self.defaultsKey)
        return id
    }
}
