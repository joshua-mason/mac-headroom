import AppKit
import SwiftUI

struct MenuLabel: View {
    @ObservedObject var store: Store
    var body: some View {
        if let d = store.status?.disk {
            Text("\(Image(systemName: "internaldrive")) \(formatBytes(d.free))")
        } else {
            Image(systemName: "internaldrive")
        }
    }
}

struct MenuView: View {
    @EnvironmentObject var store: Store
    @Environment(\.openWindow) private var openWindow
    @State private var confirmingClean = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            header
            if store.status?.findings?.fullDiskAccess == false { privacyNote }
            Divider()
            if store.scanning {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Looking at your whole disk. This takes a minute or two.")
                        .font(.callout).foregroundStyle(.secondary)
                }
            } else if let done = store.cleaned {
                cleanedSummary(done)
            } else if store.status?.findings == nil {
                Text("Scan to find out what you can safely free.")
                    .font(.callout).foregroundStyle(.secondary)
            } else {
                freeableSection
                lookSection
            }
            if let error = store.error {
                Text(error).font(.caption).foregroundStyle(.red).lineLimit(3)
            }
            Divider()
            footer
        }
        .padding(16)
        .frame(width: 340)
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let d = store.status?.disk {
                HStack(alignment: .firstTextBaseline) {
                    Text("\(formatBytes(d.free)) free").font(.system(size: 26, weight: .semibold))
                    Spacer()
                    Text(Level(freePercent: store.freePercent).words)
                        .font(.caption.weight(.semibold))
                        .padding(.horizontal, 8).padding(.vertical, 3)
                        .overlay(Capsule().stroke(.secondary.opacity(0.5)))
                }
                ProgressView(value: 1 - store.freePercent / 100)
                Text("\(Int(store.freePercent.rounded()))% of your \(formatBytes(d.total)) disk")
                    .font(.caption).foregroundStyle(.secondary)
            } else {
                Text("Reading the disk…").foregroundStyle(.secondary)
            }
        }
    }

    private var privacyNote: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text("Some folders, including your Trash, were hidden from the scan.")
                .font(.caption.weight(.semibold))
            Button("Allow Full Disk Access…") {
                if let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles") {
                    NSWorkspace.shared.open(url)
                }
            }
            .buttonStyle(.link).font(.caption)
        }
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 6).fill(Color.orange.opacity(0.12)))
    }

    private var freeableSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            if store.freeable.isEmpty {
                Text("Nothing obvious to clear right now.").font(.callout)
            } else {
                Text("You can safely free about \(formatBytes(store.freeableTotal))")
                    .font(.headline)
                ForEach(store.freeable.prefix(4)) { item in
                    HStack(alignment: .firstTextBaseline) {
                        VStack(alignment: .leading, spacing: 1) {
                            Text(item.title).font(.callout)
                            if let app = item.blockedBy {
                                Text("Quit \(app) first").font(.caption).foregroundStyle(.orange)
                            }
                        }
                        Spacer()
                        Text((item.upperBound ? "up to " : "") + formatBytes(item.bytes ?? 0))
                            .font(.callout.monospacedDigit()).foregroundStyle(.secondary)
                    }
                    .help(item.plain)
                }
                if store.freeable.count > 4 {
                    Text("and \(store.freeable.count - 4) more").font(.caption).foregroundStyle(.secondary)
                }
                cleanButton
            }
        }
    }

    @ViewBuilder private var cleanButton: some View {
        if store.cleaning {
            HStack(spacing: 8) { ProgressView().controlSize(.small); Text("Clearing…").font(.callout) }
        } else if confirmingClean {
            VStack(alignment: .leading, spacing: 6) {
                Text("Remove these caches and leftovers? Apps recreate them as needed.")
                    .font(.caption).foregroundStyle(.secondary)
                HStack {
                    Button("Remove") {
                        confirmingClean = false
                        Task { await store.clean() }
                    }
                    .keyboardShortcut(.defaultAction)
                    Button("Cancel") { confirmingClean = false }
                }
            }
        } else {
            Button("Clear these…") { confirmingClean = true }
        }
    }

    private var lookSection: some View {
        let items = (store.status?.findings?.detections ?? []).prefix(3)
        return VStack(alignment: .leading, spacing: 4) {
            if !items.isEmpty {
                Text("Worth a look").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                ForEach(Array(items)) { d in
                    HStack {
                        Text(d.title).font(.callout).lineLimit(1)
                        Spacer()
                        Text(formatBytes(d.bytes)).font(.callout.monospacedDigit()).foregroundStyle(.secondary)
                    }
                    .help(d.how)
                }
            }
        }
    }

    private func cleanedSummary(_ done: CleanResult) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Freed \(formatBytes(done.bytes)) that could be measured.").font(.headline)
            if let after = done.freeAfter {
                Text("\(formatBytes(after)) free now. Scan again to see what else could go.")
                    .font(.callout).foregroundStyle(.secondary)
            }
        }
    }

    private var footer: some View {
        HStack {
            Button(store.status?.findings == nil ? "Scan" : "Scan again") {
                Task { await store.scan() }
            }
            .disabled(store.scanning || store.cleaning)
            Button("Full report") {
                openWindow(id: "report")
                NSApp.activate(ignoringOtherApps: true)
            }
            Spacer()
            Button("Quit") { NSApp.terminate(nil) }
        }
    }
}
