// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "MacHeadroom",
    platforms: [.macOS(.v13)],
    targets: [
        // Everything visible, shared by the app and the snapshot tool.
        .target(name: "MacHeadroomUI", path: "Sources/MacHeadroomUI"),
        .executableTarget(name: "MacHeadroom", dependencies: ["MacHeadroomUI"], path: "Sources/MacHeadroom"),
        // Renders the menu to a PNG, so the design can be checked without
        // opening the menu bar by hand or needing screen recording permission.
        .executableTarget(name: "MenuSnapshot", dependencies: ["MacHeadroomUI"], path: "Sources/MenuSnapshot"),
    ]
)
