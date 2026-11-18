// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "SilasCore",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [.library(name: "SilasCore", targets: ["SilasCore"])],
    targets: [
        .target(name: "SilasCore"),
        .testTarget(name: "SilasCoreTests", dependencies: ["SilasCore"]),
    ]
)
