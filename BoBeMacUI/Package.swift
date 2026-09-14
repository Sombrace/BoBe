// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "BoBe",
    defaultLocalization: "en",
    platforms: [
        .macOS(.v15)
    ],
    dependencies: [
        .package(url: "https://github.com/sparkle-project/Sparkle", from: "2.9.4"),
        .package(url: "https://github.com/gonzalezreal/textual", exact: "0.5.0"),
        // FluidAudio — Apple Silicon ANE Parakeet/Nemotron ASR + Silero VAD.
        // Mode B client-side speech recognition.
        // Exact pin: FluidAudio is pre-1.0 and minor releases remove APIs.
        .package(url: "https://github.com/FluidInference/FluidAudio.git", exact: "0.15.7"),
    ],
    targets: [
        .executableTarget(
            name: "BoBe",
            dependencies: [
                .product(name: "Sparkle", package: "Sparkle"),
                .product(name: "Textual", package: "textual"),
                .product(name: "FluidAudio", package: "FluidAudio"),
            ],
            path: "BoBe",
            exclude: [
                "Resources/Info.plist"
            ],
            resources: [
                .process("Resources")
            ],
            swiftSettings: [
                .define("SPM_BUILD")
            ],
            linkerSettings: [
                .unsafeFlags([
                    "-Xlinker", "-rpath",
                    "-Xlinker", "@executable_path/../Frameworks",
                ])
            ]
        ),
        .testTarget(
            name: "BoBeTests",
            dependencies: ["BoBe"],
            path: "Tests/BoBeTests"
        )
    ]
)
