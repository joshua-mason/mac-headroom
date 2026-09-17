import AppKit
import SwiftUI

public struct MenuLabel: View {
    @ObservedObject var store: Store
    public init(store: Store) { self.store = store }

    public var body: some View {
        if let d = store.status?.disk {
            Text("\(Image(systemName: "internaldrive")) \(formatBytes(d.free))")
        } else {
            Image(systemName: "internaldrive")
        }
    }
}

public struct MenuView: View {
    @EnvironmentObject var store: Store
    @Environment(\.openWindow) private var openWindow
    /// A theme injected by the snapshot tool wins; otherwise the one picked in
    /// the menu, remembered between launches. Never the see-through system
    /// look: over the menu's frosted material its greys looked muddy.
    @Environment(\.theme) private var injected
    @AppStorage("theme") private var chosen = Theme.clean.name
    private var theme: Theme {
        injected.name != Theme.system.name ? injected : (Theme.named(chosen) ?? .clean)
    }
    @State private var confirming: Bool

    public init(startConfirming: Bool = false) {
        _confirming = State(initialValue: startConfirming)
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            header
            content
            if let error = store.error {
                Text(error)
                    .font(.system(size: 11))
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
            VStack(spacing: 10) {
                Rectangle().fill(theme.divider).frame(height: 1)
                footer
            }
        }
        .padding(.horizontal, 22)
        .padding(.top, 22)
        .padding(.bottom, 14)
        .frame(width: 360)
        .foregroundStyle(theme.text)
        .background(theme.background)
        .environment(\.theme, theme)
        .environment(\.colorScheme, theme.scheme)
    }

    // MARK: Header

    private var level: Level { Level(freePercent: store.freePercent) }

    private var header: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let d = store.status?.disk {
                HStack(alignment: .center) {
                    HStack(alignment: .firstTextBaseline, spacing: 6) {
                        Text(formatBytes(d.free))
                            .font(.system(size: 34, weight: .semibold, design: .rounded))
                        Text("free")
                            .font(.system(size: 15, weight: .medium, design: .rounded))
                            .foregroundStyle(theme.secondary)
                    }
                    Spacer()
                    StatusBadge(level: level)
                }
                UsageBar(fraction: 1 - store.freePercent / 100, tint: level.bar(theme))
                Text("\(formatBytes(d.total - d.free)) used of \(formatBytes(d.total))")
                    .font(.system(size: 11))
                    .foregroundStyle(theme.secondary)
            } else {
                Text("Reading your disk…")
                    .font(.system(size: 15, weight: .medium, design: .rounded))
                    .foregroundStyle(theme.secondary)
            }
        }
    }

    // MARK: Content

    @ViewBuilder private var content: some View {
        if store.scanning {
            card {
                HStack(spacing: 12) {
                    ProgressView().controlSize(.small)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Scanning your disk").font(.system(size: 13, weight: .semibold))
                        Text("This takes a minute or two.").font(.system(size: 11)).foregroundStyle(theme.secondary)
                    }
                }
            }
        } else if let done = store.cleaned {
            card {
                HStack(spacing: 12) {
                    Image(systemName: "checkmark.circle.fill")
                        .font(.system(size: 24))
                        .foregroundStyle(theme.good)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Freed \(formatBytes(done.bytes))").font(.system(size: 13, weight: .semibold))
                        Text(done.freeAfter.map { "\(formatBytes($0)) free now" } ?? "Done")
                            .font(.system(size: 11)).foregroundStyle(theme.secondary)
                    }
                }
            }
        } else if store.status?.findings == nil {
            card {
                VStack(alignment: .leading, spacing: 12) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("See what you can safely clear").font(.system(size: 13, weight: .semibold))
                        Text("A scan looks at your whole disk for caches and leftovers that are safe to remove.")
                            .font(.system(size: 11)).foregroundStyle(theme.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Button("Scan now") { Task { await store.scan() } }
                        .buttonStyle(PillButtonStyle(prominent: true, theme: theme))
                }
            }
        } else {
            freeable
        }
    }

    private var freeable: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .firstTextBaseline) {
                Text("Safe to clear").font(.system(size: 13, weight: .semibold))
                Spacer()
                if let at = store.status?.findings?.at {
                    Text("Scanned \(relativeTime(at))").font(.system(size: 11)).foregroundStyle(theme.tertiary)
                }
            }
            if store.freeable.isEmpty {
                Text("Nothing worth clearing right now.")
                    .font(.system(size: 13)).foregroundStyle(theme.secondary)
            } else {
                VStack(spacing: 0) {
                    ForEach(store.freeable.prefix(5)) { ItemRow(item: $0) }
                }
                .padding(4)
                .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(theme.card))
                .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(theme.cardStroke ?? .clear))
                clearControls
            }
        }
    }

    @ViewBuilder private var clearControls: some View {
        let blocked = store.blockedApps
        VStack(spacing: 10) {
            if store.cleaning {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Clearing…").font(.system(size: 13)).foregroundStyle(theme.secondary)
                }
                .frame(maxWidth: .infinity, minHeight: 32)
            } else if confirming {
                Text("Apps and tools recreate these when they need them. Nothing personal is removed.")
                    .font(.system(size: 11)).foregroundStyle(theme.secondary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 8) {
                    Button("Cancel") { confirming = false }
                        .buttonStyle(PillButtonStyle(prominent: false, theme: theme))
                    Button("Clear \(formatBytes(store.clearableNow))") {
                        confirming = false
                        Task { await store.clean() }
                    }
                    .buttonStyle(PillButtonStyle(prominent: true, theme: theme))
                    .keyboardShortcut(.defaultAction)
                }
            } else {
                Button("Clear \(formatBytes(store.clearableNow))") { confirming = true }
                    .buttonStyle(PillButtonStyle(prominent: true, theme: theme))
                    .disabled(store.clearableNow == 0)
                if !blocked.names.isEmpty {
                    Text("Quit \(blocked.names.joined(separator: " and ")) to clear another \(formatBytes(blocked.bytes)).")
                        .font(.system(size: 11)).foregroundStyle(theme.secondary)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    // MARK: Footer

    private var footer: some View {
        HStack(spacing: 2) {
            FooterButton(title: "Scan", systemImage: "arrow.clockwise") { Task { await store.scan() } }
                .disabled(store.scanning || store.cleaning)
            FooterButton(title: "Full report", systemImage: "doc.text.magnifyingglass") {
                openWindow(id: "report")
                NSApp.activate(ignoringOtherApps: true)
            }
            Spacer()
            // Temporary, while a look is being chosen: try each one in the real menu.
            Menu {
                ForEach(Theme.candidates) { t in
                    Button {
                        chosen = t.name
                    } label: {
                        if t.name == theme.name {
                            Label(t.name, systemImage: "checkmark")
                        } else {
                            Text(t.name)
                        }
                    }
                }
            } label: {
                Image(systemName: "paintpalette")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .foregroundStyle(theme.secondary)
            .help("Try another look")
            FooterButton(title: "Quit", systemImage: "power") { NSApp.terminate(nil) }
        }
    }

    private func card<Content: View>(@ViewBuilder _ content: () -> Content) -> some View {
        content()
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(theme.card))
            .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(theme.cardStroke ?? .clear))
    }
}

// MARK: - Pieces

struct StatusBadge: View {
    let level: Level
    @Environment(\.theme) private var theme
    var body: some View {
        HStack(spacing: 6) {
            Circle().fill(level.dot(theme)).frame(width: 7, height: 7)
            Text(level.words).font(.system(size: 12, weight: .medium))
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(Capsule().fill(theme.badge))
        .overlay(Capsule().stroke(theme.cardStroke ?? .clear))
    }
}

struct UsageBar: View {
    let fraction: Double
    let tint: Color
    @Environment(\.theme) private var theme
    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                Capsule().fill(theme.track)
                Capsule()
                    .fill(LinearGradient(colors: [tint.opacity(0.75), tint], startPoint: .leading, endPoint: .trailing))
                    .frame(width: max(8, geo.size.width * min(max(fraction, 0), 1)))
            }
        }
        .frame(height: 8)
    }
}

struct ItemRow: View {
    let item: Estimate
    @Environment(\.theme) private var theme
    @State private var hovering = false

    var body: some View {
        HStack(spacing: 11) {
            ZStack {
                RoundedRectangle(cornerRadius: 7, style: .continuous).fill(theme.iconTile)
                Image(systemName: symbol(for: item.name))
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(theme.icon)
            }
            .frame(width: 28, height: 28)
            VStack(alignment: .leading, spacing: 1) {
                Text(shortTitle(for: item)).font(.system(size: 13)).lineLimit(1)
                if let app = item.blockedBy {
                    Text("Needs \(app) closed").font(.system(size: 11)).foregroundStyle(theme.secondary)
                }
            }
            Spacer(minLength: 8)
            HStack(alignment: .firstTextBaseline, spacing: 3) {
                if item.upperBound {
                    Text("up to").font(.system(size: 11)).foregroundStyle(theme.secondary)
                }
                Text(formatBytes(item.bytes ?? 0))
                    .font(.system(size: 13, weight: .medium).monospacedDigit())
                    .foregroundStyle(item.blockedBy == nil ? theme.text : theme.secondary)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(RoundedRectangle(cornerRadius: 9, style: .continuous).fill(hovering ? theme.hover : .clear))
        .onHover { hovering = $0 }
        .help("\(item.title). \(item.plain)")
    }
}

/// A full-width rounded button that looks the same whether or not the menu's
/// window is active, unlike the system's prominent style.
struct PillButtonStyle: ButtonStyle {
    let prominent: Bool
    let theme: Theme
    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(size: 13, weight: .semibold))
            .frame(maxWidth: .infinity, minHeight: 34)
            .foregroundStyle(prominent ? theme.accentText : theme.text)
            .background(
                RoundedRectangle(cornerRadius: 9, style: .continuous)
                    .fill(prominent ? theme.accent : theme.quietButton)
            )
            .opacity(isEnabled ? (configuration.isPressed ? 0.8 : 1) : 0.45)
            .contentShape(Rectangle())
    }
}

struct FooterButton: View {
    @Environment(\.theme) private var theme
    let title: String
    let systemImage: String
    let action: () -> Void
    @State private var hovering = false
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        Button(action: action) {
            Label(title, systemImage: systemImage)
                .font(.system(size: 12, weight: .medium))
                .padding(.horizontal, 8)
                .padding(.vertical, 5)
                .background(RoundedRectangle(cornerRadius: 6, style: .continuous)
                    .fill(hovering && isEnabled ? theme.hover : .clear))
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .foregroundStyle(theme.secondary)
        .onHover { hovering = $0 }
    }
}

// MARK: - Menu wording

/// Menus want short names. The full plain title stays in the tooltip.
func shortTitle(for item: Estimate) -> String {
    switch item.name {
    case "chrome-cache": return "Chrome cache"
    case "spotify-cache": return "Spotify cache"
    case "npm-cache": return "JavaScript packages"
    case "pnpm-store": return "Unused pnpm packages"
    case "uv-cache": return "Python packages"
    case "language-caches": return "Package downloads"
    case "go-build-cache": return "Go build cache"
    case "homebrew": return "Homebrew leftovers"
    case "xcode-derived-data": return "Xcode build files"
    case "updater-leftovers": return "Old app updates"
    case "whatsapp-orphans": return "Forgotten WhatsApp files"
    default: return item.title
    }
}

func symbol(for name: String) -> String {
    switch name {
    case "chrome-cache": return "globe"
    case "spotify-cache": return "music.note"
    case "npm-cache", "pnpm-store": return "shippingbox"
    case "uv-cache", "language-caches": return "curlybraces"
    case "go-build-cache": return "gearshape.2"
    case "homebrew": return "mug"
    case "xcode-derived-data": return "hammer"
    case "updater-leftovers": return "arrow.down.circle"
    case "whatsapp-orphans": return "photo.on.rectangle"
    default: return "sparkles"
    }
}

func relativeTime(_ epoch: UInt64) -> String {
    let formatter = RelativeDateTimeFormatter()
    formatter.unitsStyle = .full
    return formatter.localizedString(for: Date(timeIntervalSince1970: TimeInterval(epoch)), relativeTo: Date())
}
