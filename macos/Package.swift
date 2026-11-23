// swift-tools-version:6.0
import PackageDescription

let package = Package(
    name: "Streamprobe",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "Streamprobe", targets: ["StreamprobeApp"]),
    ],
    targets: [
        .target(name: "StreamprobeKit"),
        .executableTarget(name: "StreamprobeApp", dependencies: ["StreamprobeKit"]),
        .testTarget(
            name: "StreamprobeKitTests",
            dependencies: ["StreamprobeKit"],
            resources: [.copy("Fixtures")]
        ),
    ]
)
