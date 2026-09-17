import Foundation
import SwiftUI

@MainActor
public final class Store: ObservableObject {
    @Published var status: Status?
    @Published var error: String?
    @Published var scanning = false
    @Published var cleaning = false
    /// Set after a clean, until the next scan: the saved sizes are out of date.
    @Published var cleaned: CleanResult?
    @Published var reportURL: URL?

    private var timer: Timer?

    public init(autoRefresh: Bool = true) {
        guard autoRefresh else { return }
        Task { await refresh() }
        timer = Timer.scheduledTimer(withTimeInterval: 15 * 60, repeats: true) { [weak self] _ in
            Task { await self?.refresh() }
        }
    }

    var freePercent: Double {
        guard let d = status?.disk, d.total > 0 else { return 0 }
        return 100 * Double(d.free) / Double(d.total)
    }

    /// Items worth listing: at least 1 MB, largest first.
    var freeable: [Estimate] {
        (status?.findings?.reclaimable ?? [])
            .filter { ($0.bytes ?? 0) >= 1 << 20 }
            .sorted { ($0.bytes ?? 0) > ($1.bytes ?? 0) }
    }

    var freeableTotal: UInt64 { freeable.reduce(0) { $0 + ($1.bytes ?? 0) } }

    /// What a clear frees now. Items whose app is open are skipped by the CLI.
    var clearableNow: UInt64 { freeable.filter { $0.blockedBy == nil }.reduce(0) { $0 + ($1.bytes ?? 0) } }

    /// Apps that must be quit to clear everything, and how much that would add.
    var blockedApps: (names: [String], bytes: UInt64) {
        let blocked = freeable.filter { $0.blockedBy != nil }
        var names: [String] = []
        for item in blocked { if let n = item.blockedBy, !names.contains(n) { names.append(n) } }
        return (names, blocked.reduce(0) { $0 + ($1.bytes ?? 0) })
    }

    public func refresh() async {
        do {
            status = try Headroom.decode(Status.self, from: try await Headroom.run(["status"]))
            error = nil
        } catch {
            self.error = error.localizedDescription
        }
    }

    func scan() async {
        scanning = true
        defer { scanning = false }
        do {
            _ = try await Headroom.run(["scan"])
            cleaned = nil
            reportURL = nil
            await refresh()
        } catch {
            self.error = error.localizedDescription
        }
    }

    func clean() async {
        cleaning = true
        defer { cleaning = false }
        do {
            cleaned = try Headroom.decode(CleanResult.self, from: try await Headroom.run(["clean", "--yes"]))
            await refresh()
        } catch {
            self.error = error.localizedDescription
        }
    }

    func makeReport() async {
        do {
            let file = try Headroom.decode(ReportFile.self, from: try await Headroom.run(["report"]))
            reportURL = URL(fileURLWithPath: file.path)
        } catch {
            self.error = error.localizedDescription
        }
    }
}
