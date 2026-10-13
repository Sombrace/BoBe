import Foundation

struct CommandLineToolInstaller {
    static var launcherURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".local/bin/bobe")
    }

    static func install(bundleURL: URL = Bundle.main.bundleURL) throws -> URL {
        let daemon = bundleURL.appendingPathComponent("Contents/MacOS/bobe-daemon")
        guard FileManager.default.isExecutableFile(atPath: daemon.path) else {
            throw CommandLineToolInstallerError.daemonMissing
        }
        let launcher = self.launcherURL
        try FileManager.default.createDirectory(
            at: launcher.deletingLastPathComponent(),
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o755]
        )
        let script = "#!/bin/sh\nexec \"\(daemon.path.replacingOccurrences(of: "\\\"", with: "\\\\\\\""))\" \"$@\"\n"
        try script.write(to: launcher, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: launcher.path)
        return launcher
    }
}

enum CommandLineToolInstallerError: LocalizedError {
    case daemonMissing

    var errorDescription: String? {
        switch self {
        case .daemonMissing: "The bundled BoBe command-line tool is missing. Reinstall BoBe from the DMG."
        }
    }
}
