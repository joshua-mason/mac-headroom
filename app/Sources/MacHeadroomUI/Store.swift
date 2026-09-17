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
    /// Changes whenever the report file is rewritten, so the window reloads it.
    @Published var reportVersion = 0
    private var reportTask: Task<Void, Never>?
    /// Which stage a scan is on: 0 checking the disk, 1 measuring folders,
    /// 2 working out what is safe to clear.
    @Published var scanStep: Int?
    @Published var scanStarted: Date?
    /// Advances while scanning, to animate the menu bar item.
    @Published var spinnerFrame = 0
    private var spinnerTimer: Timer?

    private var timer: Timer?

    public init(autoRefresh: Bool = true) {
        guard autoRefresh else { return }
        Task {
            await refresh()
            await makeReport()
        }
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

    func scan(skipProtected: Bool = false) async {
        scanning = true
        scanStep = 0
        scanStarted = Date()
        startSpinner()
        defer {
            scanning = false
            scanStep = nil
            scanStarted = nil
            stopSpinner()
        }
        do {
            let stages = ["disk": 0, "folders": 1, "cleaners": 2]
            _ = try await Headroom.run(skipProtected ? ["scan", "--skip-protected"] : ["scan"]) { [weak self] stage in
                Task { @MainActor in
                    if let step = stages[stage] { self?.scanStep = step }
                }
            }
            cleaned = nil
            await refresh()
            prepareReport()
        } catch {
            self.error = error.localizedDescription
        }
    }

    private func startSpinner() {
        spinnerTimer?.invalidate()
        spinnerTimer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.spinnerFrame += 1 }
        }
    }

    private func stopSpinner() {
        spinnerTimer?.invalidate()
        spinnerTimer = nil
    }

    /// For the snapshot tool: show a scan in progress without running one.
    public func previewScanning(step: Int, secondsIn: TimeInterval) {
        scanning = true
        scanStep = step
        scanStarted = Date().addingTimeInterval(-secondsIn)
    }

    func clean() async {
        cleaning = true
        defer { cleaning = false }
        do {
            cleaned = try Headroom.decode(CleanResult.self, from: try await Headroom.run(["clean", "--yes"]))
            await refresh()
            prepareReport()
        } catch {
            self.error = error.localizedDescription
        }
    }

    /// Write a fresh report. Without Full Disk Access the CLI is told to leave
    /// protected folders alone, since touching one would make macOS stop and
    /// ask, holding the report up until someone answers. Overlapping requests
    /// share one run.
    func makeReport() async {
        if let running = reportTask {
            await running.value
            return
        }
        let skip = status?.fullDiskAccess != true
        let task = Task { @MainActor in
            do {
                let file = try Headroom.decode(ReportFile.self,
                                               from: try await Headroom.run(skip ? ["report", "--skip-protected"] : ["report"]))
                self.reportURL = URL(fileURLWithPath: file.path)
                self.reportVersion += 1
            } catch {
                self.error = error.localizedDescription
            }
        }
        reportTask = task
        await task.value
        reportTask = nil
    }

    /// Keep a report ready, so opening the window shows one straight away.
    func prepareReport() {
        Task { await makeReport() }
    }
}
