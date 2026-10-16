@testable import BoBe
import Foundation
import Testing

@Suite("Command-line tool installer")
struct CommandLineToolInstallerTests {
    @Test
    func launcherLinksTheBundledToolWithoutInterpretingItsPath() throws {
        let root = try Self.makeRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let bundle = root.appending(
            path: #"Bo$Be $(touch substituted) `touch backticked` "quoted" 'single' \ "#
                + "\n newline.app",
            directoryHint: .isDirectory
        )
        let tool = try Self.makeTool(in: bundle)
        let launcher = root.appending(path: "bin/bobe")

        let installed = try CommandLineToolInstaller.install(bundleURL: bundle, launcher: launcher)

        #expect(installed == launcher)
        #expect(try FileManager.default.destinationOfSymbolicLink(atPath: launcher.path) == tool.path)
        let output = try Self.run(launcher, arguments: ["status", "two words", "$HOME"], in: root)
        #expect(output == "status\ntwo words\n$HOME\n")
        #expect(!FileManager.default.fileExists(atPath: root.appending(path: "substituted").path))
        #expect(!FileManager.default.fileExists(atPath: root.appending(path: "backticked").path))
    }

    @Test
    func reinstallAtomicallyReplacesAnExistingLauncher() throws {
        let root = try Self.makeRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let tool = try Self.makeTool(in: root.appending(path: "BoBe.app", directoryHint: .isDirectory))
        let launcher = root.appending(path: "bin/bobe")
        try FileManager.default.createDirectory(
            at: launcher.deletingLastPathComponent(), withIntermediateDirectories: true
        )
        try Data("#!/bin/sh\nexec /nonexistent \"$@\"\n".utf8).write(to: launcher)

        _ = try CommandLineToolInstaller.install(
            bundleURL: root.appending(path: "BoBe.app", directoryHint: .isDirectory),
            launcher: launcher
        )

        #expect(try FileManager.default.destinationOfSymbolicLink(atPath: launcher.path) == tool.path)
        #expect(try FileManager.default.contentsOfDirectory(atPath: launcher.deletingLastPathComponent().path) == ["bobe"])
    }

    @Test
    func missingBundledToolLeavesNoLauncher() throws {
        let root = try Self.makeRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let launcher = root.appending(path: "bin/bobe")

        #expect(throws: CommandLineToolInstallerError.self) {
            try CommandLineToolInstaller.install(
                bundleURL: root.appending(path: "BoBe.app", directoryHint: .isDirectory),
                launcher: launcher
            )
        }
        #expect(!FileManager.default.fileExists(atPath: launcher.path))
    }

    private static func makeRoot() throws -> URL {
        let root = FileManager.default.temporaryDirectory
            .appending(path: "bobe-cli-tests-\(UUID().uuidString)", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        return root
    }

    private static func makeTool(in bundle: URL) throws -> URL {
        let directory = bundle.appending(path: "Contents/MacOS", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let tool = directory.appending(path: "bobe-daemon")
        try Data("#!/bin/sh\nprintf '%s\\n' \"$@\"\n".utf8).write(to: tool)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: tool.path)
        return tool
    }

    private static func run(_ executable: URL, arguments: [String], in directory: URL) throws -> String {
        let process = Process()
        let output = Pipe()
        process.executableURL = executable
        process.arguments = arguments
        process.currentDirectoryURL = directory
        process.standardOutput = output
        try process.run()
        process.waitUntilExit()
        #expect(process.terminationStatus == 0)
        let data = output.fileHandleForReading.readDataToEndOfFile()
        return String(decoding: data, as: UTF8.self)
    }
}
