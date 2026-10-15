// swift-tools-version: 6.0
import PackageDescription

let packageDirectory = Context.packageDirectory

let package = Package(
    name: "BoBe",
    defaultLocalization: "en",
    platforms: [
        .macOS("26.0")
    ],
    dependencies: [
        .package(url: "https://github.com/sparkle-project/Sparkle", exact: "2.9.6"),
        .package(url: "https://github.com/gonzalezreal/textual", exact: "0.5.0"),
        .package(url: "https://github.com/FluidInference/FluidAudio.git", exact: "0.15.6"),
    ],
    targets: [
        .target(
            name: "PairingClient",
            path: "PairingClient",
            exclude: ["PairingClient-Bridging-Header.h"],
            swiftSettings: [
                .unsafeFlags([
                    "-import-objc-header", "\(packageDirectory)/PairingClient/PairingClient-Bridging-Header.h",
                ])
            ],
            linkerSettings: [
                .unsafeFlags([
                    "-L", "\(packageDirectory)/../PairingFFI/target/release",
                    "-lbobe_pairing_ffi",
                ])
            ]
        ),
        .executableTarget(
            name: "BoBe",
            dependencies: [
                "PairingClient",
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
                .define("SPM_BUILD"),
                .unsafeFlags([
                    "-warnings-as-errors",
                ])
            ],
            linkerSettings: [
                .unsafeFlags([
                    "-framework", "Security",
                    "-framework", "SystemConfiguration",
                    "-Xlinker", "-rpath",
                    "-Xlinker", "@executable_path/../Frameworks",
                ])
            ]
        )
    ]
)
