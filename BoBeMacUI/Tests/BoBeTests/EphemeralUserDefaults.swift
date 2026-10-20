import Foundation

nonisolated final class EphemeralUserDefaults: UserDefaults, @unchecked Sendable {
    let suiteName: String
    let persistentFileURL: URL
    private let directory: URL

    init?(prefix: String) {
        precondition(!prefix.isEmpty && !prefix.contains("/"))
        let template = FileManager.default.temporaryDirectory
            .appending(path: "\(prefix).XXXXXX", directoryHint: .isDirectory)
        var bytes = Array(template.path.utf8CString)
        guard mkdtemp(&bytes) != nil else { return nil }
        guard let path = String(bytes: bytes.dropLast().map { UInt8(bitPattern: $0) }, encoding: .utf8) else {
            _ = rmdir(bytes)
            return nil
        }
        directory = URL(fileURLWithPath: path, isDirectory: true)
        suiteName = directory.appending(path: "preferences").path
        persistentFileURL = directory.appending(path: "preferences.plist")
        super.init(suiteName: suiteName)
    }

    deinit {
        do {
            try cleanUp()
        } catch {
            NSLog("Ephemeral preferences cleanup failed for %@: %@", suiteName, String(describing: error))
        }
    }

    func cleanUp() throws {
        removePersistentDomain(forName: suiteName)
        guard synchronize() else {
            throw CocoaError(.fileWriteUnknown)
        }
        do {
            try FileManager.default.removeItem(at: directory)
        } catch CocoaError.fileNoSuchFile {
            return
        }
    }
}
