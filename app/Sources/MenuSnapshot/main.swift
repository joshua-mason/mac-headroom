import AppKit
import MacHeadroomUI
import SwiftUI

// Usage:
//   MenuSnapshot <out.png> [--dark] [--confirming] [--theme <name>]
//   MenuSnapshot <out.png> --sheet      every candidate theme side by side
// Loads real status through the CLI (set MAC_HEADROOM_CLI).
let args = CommandLine.arguments
let outPath = args.count > 1 ? args[1] : "menu.png"
let dark = args.contains("--dark")
let confirming = args.contains("--confirming")
let sheet = args.contains("--sheet")
let themeName = args.firstIndex(of: "--theme").flatMap { args.indices.contains($0 + 1) ? args[$0 + 1] : nil }

@MainActor
func render<V: View>(_ view: V, appearance: NSAppearance?) async throws -> Data {
    let host = NSHostingView(rootView: view)
    host.appearance = appearance
    let size = host.fittingSize
    let window = NSWindow(contentRect: NSRect(x: -30000, y: -30000, width: size.width, height: size.height),
                          styleMask: .borderless, backing: .buffered, defer: false)
    window.appearance = appearance
    window.contentView = host
    window.orderFrontRegardless()
    host.layoutSubtreeIfNeeded()
    try await Task.sleep(nanoseconds: 700_000_000)
    guard let rep = host.bitmapImageRepForCachingDisplay(in: host.bounds) else { throw NSError(domain: "snap", code: 1) }
    host.cacheDisplay(in: host.bounds, to: rep)
    guard let png = rep.representation(using: .png, properties: [:]) else { throw NSError(domain: "snap", code: 2) }
    print("rendered \(Int(size.width))x\(Int(size.height))")
    return png
}

@MainActor
func main() async throws {
    _ = NSApplication.shared
    NSApp.setActivationPolicy(.accessory)
    let store = Store(autoRefresh: false)
    await store.refresh()

    let data: Data
    if sheet {
        let view = HStack(alignment: .top, spacing: 28) {
            ForEach(Theme.candidates) { t in
                VStack(alignment: .leading, spacing: 10) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(t.name).font(.system(size: 17, weight: .semibold))
                        Text(t.note).font(.system(size: 12)).foregroundColor(Color(hex: 0x555A63))
                    }
                    .foregroundColor(Color(hex: 0x1C1F24))
                    MenuView()
                        .environmentObject(store)
                        .environment(\.theme, t)
                        .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).stroke(Color.black.opacity(0.08)))
                }
            }
        }
        .padding(36)
        .background(Color(hex: 0xD8DBE0))
        data = try await render(view, appearance: NSAppearance(named: .aqua))
    } else {
        let theme = Theme.candidates.first { $0.name.lowercased() == themeName?.lowercased() } ?? .system
        let view = MenuView(startConfirming: confirming)
            .environmentObject(store)
            .environment(\.theme, theme)
            .background(theme.name == "System" ? Color(nsColor: .windowBackgroundColor) : .clear)
        data = try await render(view, appearance: NSAppearance(named: dark ? .darkAqua : .aqua))
    }
    try data.write(to: URL(fileURLWithPath: outPath))
    print("wrote \(outPath)")
}

try await main()
