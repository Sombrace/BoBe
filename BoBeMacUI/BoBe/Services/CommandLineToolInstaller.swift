import Foundation

struct CommandLineToolInstaller {
    static var launcherURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".local/bin/bobe")
    }

    static func install(
        bundleURL: URL = Bundle.main.bundleURL,
        launcher: URL = Self.launcherURL
    ) throws -> URL {
        let executable = bundleURL.appendingPathComponent("Contents/MacOS/bobe-daemon")
        guard FileManager.default.isExecutableFile(atPath: executable.path) else {
            throw CommandLineToolInstallerError.daemonMissing
        }
        let directory = launcher.deletingLastPathComponent()
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o755]
        )
        let staged = directory.appendingPathComponent(".bobe-\(UUID().uuidString)")
        try FileManager.default.createSymbolicLink(
            atPath: staged.path,
            withDestinationPath: executable.path
        )
        guard rename(staged.path, launcher.path) == 0 else {
            let code = POSIXErrorCode(rawValue: errno) ?? .EIO
            try? FileManager.default.removeItem(at: staged)
            throw POSIXError(code)
        }
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
