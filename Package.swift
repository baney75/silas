// swift-tools-version: 5.9
// SilasCore for Apple apps that pin Silas by tag or revision:
//   .package(url: "https://github.com/baney75/silas", revision: "<sha>")
// SwiftPM reads a remote package's manifest only from the repo root, so this one points into
// ios/SilasCore. ios/SilasCore/Package.swift describes the same sources for local work there.
import PackageDescription

let package = Package(
    name: "SilasCore",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "SilasCore", targets: ["SilasCore"])],
    targets: [
        .target(
            name: "SilasCore", path: "ios/SilasCore/Sources/SilasCore",
            resources: [.copy("Resources/common-en.txt")]),
        .testTarget(
            name: "SilasCoreTests", dependencies: ["SilasCore"],
            path: "ios/SilasCore/Tests/SilasCoreTests", exclude: ["Fixtures"]),
    ]
)
