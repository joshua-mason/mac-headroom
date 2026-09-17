import AppKit
import SwiftUI

public struct MenuLabel: View {
    @ObservedObject var store: Store
    public init(store: Store) { self.store = store }

    private static let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]

    public var body: some View {
        if store.scanning {
            Text("\(Image(systemName: "internaldrive")) \(Self.spinner[store.spinnerFrame % Self.spinner.count])")
        } else if let d = store.status?.disk {
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
    /// Showing the explanation of Full Disk Access instead of starting a scan.
    @State private var askingAccess = false
    /// The person chose to scan without access; do not ask them every time.
    @AppStorage("scanWithoutAccess") private var scanWithoutAccess = false

    static let height: CGFloat = 548

    public init(startAskingAccess: Bool = false) {
        _askingAccess = State(initialValue: startAskingAccess)
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            header
            content
                .frame(maxHeight: .infinity, alignment: .top)
            if let error = store.error {
                Text(error)
                    .font(.system(size: 11))
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
            VStack(spacing: 10) {
                Rectangle().fill(theme.divider).frame(height: 1)
                footer
            }
        }
        .padding(.horizontal, 22)
        .padding(.top, 22)
        .padding(.bottom, 14)
        // One size for every state. When the menu's window changes height,
        // macOS briefly draws a ghost of its old size, so it never does.
        .frame(width: 360, height: Self.height, alignment: .top)
        .foregroundStyle(theme.text)
        .background(theme.background.ignoresSafeArea())
        .background(MenuWindowAnchor())
        .onAppear { Task { await store.refresh() } }
        .task(id: askingAccess) { await waitForAccess() }
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
        if askingAccess {
            accessCard
        } else if store.scanning {
            scanProgress
        } else if store.status?.findings == nil {
            card {
                VStack(alignment: .leading, spacing: 12) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("See what you can safely clear").font(.system(size: 13, weight: .semibold))
                        Text("A scan looks at your whole disk for caches and leftovers that are safe to remove.")
                            .font(.system(size: 11)).foregroundStyle(theme.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Button("Scan now") { startScan() }
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
                Spacer(minLength: 0)
                clearControls
            }
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    @ViewBuilder private var clearControls: some View {
        let blocked = store.blockedApps
        VStack(spacing: 10) {
            if store.cleaning {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Clearing…").font(.system(size: 13)).foregroundStyle(theme.secondary)
                }
                .frame(maxWidth: .infinity, minHeight: 34)
            } else {
                Button(store.clearableNow > 0 ? "Clear \(formatBytes(store.clearableNow))" : "Nothing to clear") {
                    Task { await store.clean() }
                }
                .buttonStyle(PillButtonStyle(prominent: true, theme: theme))
                .disabled(store.clearableNow == 0 || store.scanning)
            }
            if let done = store.cleaned, !store.cleaning {
                HStack(spacing: 6) {
                    Image(systemName: "checkmark.circle.fill").foregroundStyle(theme.good)
                    Text(done.bytes > 0 ? "Freed \(formatBytes(done.bytes))" : "Already clear, nothing more to free")
                        .foregroundStyle(theme.secondary)
                }
                .font(.system(size: 11, weight: .medium))
                .frame(maxWidth: .infinity)
            } else if !blocked.names.isEmpty {
                Text("Quit \(blocked.names.joined(separator: " and ")) to clear another \(formatBytes(blocked.bytes)).")
                    .font(.system(size: 11)).foregroundStyle(theme.secondary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    // MARK: Scanning

    private var scanProgress: some View {
        VStack(alignment: .leading, spacing: 22) {
            HStack(alignment: .firstTextBaseline) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Scanning your disk").font(.system(size: 15, weight: .semibold))
                    Text("Usually a minute or two. You can close this menu; it carries on.")
                        .font(.system(size: 11)).foregroundStyle(theme.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                Spacer(minLength: 8)
                if let started = store.scanStarted {
                    TimelineView(.periodic(from: started, by: 1)) { context in
                        Text(elapsed(from: started, to: context.date))
                            .font(.system(size: 12, weight: .medium).monospacedDigit())
                            .foregroundStyle(theme.secondary)
                    }
                }
            }
            SlidingBar()
            VStack(alignment: .leading, spacing: 14) {
                stageRow(0, "Checking the disk")
                stageRow(1, "Measuring where the space is")
                stageRow(2, "Working out what is safe to clear")
            }
        }
        .padding(18)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(theme.card))
        .overlay(RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(theme.cardStroke ?? .clear))
    }

    private func stageRow(_ index: Int, _ text: String) -> some View {
        let current = store.scanStep ?? 0
        return HStack(spacing: 10) {
            ZStack {
                if index < current {
                    Image(systemName: "checkmark.circle.fill").foregroundStyle(theme.good)
                } else if index == current {
                    ProgressView().controlSize(.mini)
                } else {
                    Image(systemName: "circle").foregroundStyle(theme.tertiary)
                }
            }
            .font(.system(size: 14))
            .frame(width: 18, height: 18)
            Text(text)
                .font(.system(size: 13, weight: index == current ? .semibold : .regular))
                .foregroundStyle(index > current ? theme.tertiary : theme.text)
        }
    }

    private func elapsed(from start: Date, to now: Date) -> String {
        let seconds = max(0, Int(now.timeIntervalSince(start)))
        return String(format: "%d:%02d", seconds / 60, seconds % 60)
    }

    // MARK: Asking for access once

    private func startScan() {
        if store.status?.fullDiskAccess == true {
            Task { await store.scan() }
        } else if scanWithoutAccess {
            Task { await store.scan(skipProtected: true) }
        } else {
            askingAccess = true
        }
    }

    /// While the explanation is showing, notice the moment access is granted in
    /// System Settings and carry straight on with a full scan.
    private func waitForAccess() async {
        guard askingAccess else { return }
        while askingAccess && !Task.isCancelled {
            try? await Task.sleep(nanoseconds: 2_000_000_000)
            await store.refresh()
            if store.status?.fullDiskAccess == true {
                askingAccess = false
                scanWithoutAccess = false
                await store.scan()
                return
            }
        }
    }

    private var accessCard: some View {
        card {
            VStack(alignment: .leading, spacing: 14) {
                HStack(spacing: 10) {
                    ZStack {
                        RoundedRectangle(cornerRadius: 8, style: .continuous).fill(theme.iconTile)
                        Image(systemName: "lock.open").font(.system(size: 14, weight: .semibold)).foregroundStyle(theme.icon)
                    }
                    .frame(width: 32, height: 32)
                    Text("Let mac-headroom see your whole disk")
                        .font(.system(size: 13, weight: .semibold))
                        .fixedSize(horizontal: false, vertical: true)
                }
                Text("Without Full Disk Access, macOS stops to ask about Photos, Documents, Downloads and other apps' data one folder at a time. Turning it on once covers all of them, and nothing leaves your Mac.")
                    .font(.system(size: 11)).foregroundStyle(theme.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: 5) {
                    step(1, "Open System Settings")
                    step(2, "Turn on mac-headroom. If it is not listed, press + and choose it.")
                    step(3, "Come back. The scan starts on its own.")
                }
                VStack(spacing: 8) {
                    Button("Open System Settings") {
                        if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles") {
                            NSWorkspace.shared.open(url)
                        }
                    }
                    .buttonStyle(PillButtonStyle(prominent: true, theme: theme))
                    Button("Scan without it") {
                        askingAccess = false
                        scanWithoutAccess = true
                        Task { await store.scan(skipProtected: true) }
                    }
                    .buttonStyle(PillButtonStyle(prominent: false, theme: theme))
                    Text("Skips those folders, so you are not asked anything.")
                        .font(.system(size: 11)).foregroundStyle(theme.tertiary)
                        .frame(maxWidth: .infinity)
                }
            }
        }
    }

    private func step(_ n: Int, _ text: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text("\(n)")
                .font(.system(size: 10, weight: .bold))
                .foregroundStyle(theme.accentText)
                .frame(width: 16, height: 16)
                .background(Circle().fill(theme.accent))
            Text(text).font(.system(size: 11)).foregroundStyle(theme.text)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: Footer

    private var footer: some View {
        HStack(spacing: 2) {
            FooterButton(title: "Scan", systemImage: "arrow.clockwise") { startScan() }
                .disabled(store.scanning || store.cleaning)
            FooterButton(title: "Full report", systemImage: "doc.text.magnifyingglass") {
                MenuWindowAnchor.close()
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

/// An indeterminate bar: a highlight sliding along the track while work goes on.
struct SlidingBar: View {
    @Environment(\.theme) private var theme

    var body: some View {
        TimelineView(.animation) { context in
            GeometryReader { geo in
                let cycle = 1.6
                let phase = context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: cycle) / cycle
                let width = geo.size.width * 0.35
                ZStack(alignment: .leading) {
                    Capsule().fill(theme.track)
                    Capsule()
                        .fill(theme.accent)
                        .frame(width: width)
                        .offset(x: -width + (geo.size.width + width) * phase)
                }
                .clipShape(Capsule())
            }
        }
        .frame(height: 6)
    }
}

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

// MARK: - Closing the menu

/// Holds the menu's window so an action that opens another window can close
/// the menu first. It deliberately does not move or resize the window: an
/// earlier version repositioned it after resizes, and that pulled the content
/// away from macOS's own window background. The menu now never changes size.
struct MenuWindowAnchor: NSViewRepresentable {
    private static weak var current: NSWindow?

    static func close() { current?.orderOut(nil) }

    func makeNSView(context: Context) -> NSView { AnchorView() }
    func updateNSView(_ nsView: NSView, context: Context) {}

    final class AnchorView: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let window { MenuWindowAnchor.current = window }
        }
    }
}
