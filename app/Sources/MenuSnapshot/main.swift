import AppKit
import MacHeadroomUI
import SwiftUI

// Usage: MenuSnapshot <out.png> [--dark]
// Loads real status through the CLI (set MAC_HEADROOM_CLI) and renders the menu.
let args = CommandLine.arguments
let outPath = args.count > 1 ? args[1] : "menu.png"
let dark = args.contains("--dark")
let confirming = args.contains("--confirming")

@MainActor
func snapshot() async throws {
    _ = NSApplication.shared
    NSApp.setActivationPolicy(.accessory)
    let store = Store(autoRefresh: false)
    await store.refresh()

    let appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
    let root = MenuView(startConfirming: confirming)
        .environmentObject(store)
        .background(Color(nsColor: .windowBackgroundColor))
    let host = NSHostingView(rootView: root)
    host.appearance = appearance
    let size = host.fittingSize
    let window = NSWindow(contentRect: NSRect(x: -20000, y: -20000, width: size.width, height: size.height),
                          styleMask: .borderless, backing: .buffered, defer: false)
    window.appearance = appearance
    window.contentView = host
    window.orderFrontRegardless()
    host.layoutSubtreeIfNeeded()
    try await Task.sleep(nanoseconds: 600_000_000)

    guard let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) else {
        throw NSError(domain: "MenuSnapshot", code: 1)
    }
    host.cacheDisplay(in: host.bounds, to: rep)
    guard let png = rep.representation(using: .png, properties: [:]) else {
        throw NSError(domain: "MenuSnapshot", code: 2)
    }
    try png.write(to: URL(fileURLWithPath: outPath))
    print("wrote \(outPath) \(Int(size.width))x\(Int(size.height))")
}

try await snapshot()
