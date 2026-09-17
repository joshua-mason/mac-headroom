import Foundation
import SwiftUI

/// The app never measures or deletes anything itself. It runs the mac-headroom
/// command line tool bundled inside it and reads its JSON, so the app, the CLI
/// and any AI agent using the CLI all run exactly the same code.
enum Headroom {
    struct Failure: LocalizedError {
        let message: String
        var errorDescription: String? { message }
    }

    /// Bundled in Contents/MacOS for a built app. While developing, point
    /// MAC_HEADROOM_CLI at a build, or fall back to a Homebrew install.
    static var executable: URL {
        if let bundled = Bundle.main.url(forAuxiliaryExecutable: "mac-headroom") { return bundled }
        if let dev = ProcessInfo.processInfo.environment["MAC_HEADROOM_CLI"] {
            return URL(fileURLWithPath: dev)
        }
        return URL(fileURLWithPath: "/opt/homebrew/bin/mac-headroom")
    }

    /// Run a command with --json and --no-open, off the main thread. Lines the
    /// CLI writes as `progress: <stage>` are passed to `onProgress` as they come.
    static func run(_ args: [String], onProgress: ((String) -> Void)? = nil) async throws -> Data {
        try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = executable
                process.arguments = ["--json", "--no-open"] + args
                var environment = ProcessInfo.processInfo.environment
                environment["MAC_HEADROOM_TRACE"] = "1"
                process.environment = environment
                let started = Date()
                let out = Pipe(), err = Pipe()
                process.standardOutput = out
                process.standardError = err
                do { try process.run() } catch {
                    continuation.resume(throwing: Failure(message: "Could not start mac-headroom: \(error.localizedDescription)"))
                    return
                }
                // Drain both pipes at once. Reading one to the end first can
                // deadlock if the other fills its buffer while we wait.
                var stdout = Data(), stderr = Data()
                let group = DispatchGroup()
                group.enter()
                DispatchQueue.global().async { stdout = out.fileHandleForReading.readDataToEndOfFile(); group.leave() }
                group.enter()
                DispatchQueue.global().async {
                    let handle = err.fileHandleForReading
                    var pending = Data()
                    while true {
                        let chunk = handle.availableData
                        if chunk.isEmpty { break }
                        stderr.append(chunk)
                        pending.append(chunk)
                        while let newline = pending.firstIndex(of: 0x0A) {
                            let line = String(decoding: pending[pending.startIndex..<newline], as: UTF8.self)
                            pending.removeSubrange(pending.startIndex...newline)
                            if line.hasPrefix("progress: "), let onProgress {
                                onProgress(String(line.dropFirst("progress: ".count)))
                            }
                        }
                    }
                    group.leave()
                }
                group.wait()
                process.waitUntilExit()
                log(args: args, seconds: Date().timeIntervalSince(started),
                    status: process.terminationStatus, stderr: stderr)
                if process.terminationStatus == 0 {
                    continuation.resume(returning: stdout)
                } else {
                    let text = String(decoding: stderr, as: UTF8.self)
                        .split(separator: "\n")
                        .filter { !$0.hasPrefix("progress: ") }
                        .joined(separator: "\n")
                        .trimmingCharacters(in: .whitespacesAndNewlines)
                    continuation.resume(throwing: Failure(
                        message: text.isEmpty ? "mac-headroom exited with status \(process.terminationStatus)" : text))
                }
            }
        }
    }

    /// One line per command, plus any timing lines, in
    /// ~/Library/Logs/mac-headroom-app.log, so slowness can be measured rather
    /// than guessed at.
    private static func log(args: [String], seconds: TimeInterval, status: Int32, stderr: Data) {
        let url = FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/mac-headroom-app.log")
        let stamp = ISO8601DateFormatter().string(from: Date())
        var text = "\(stamp)  \(args.joined(separator: " "))  \(String(format: "%.2f", seconds))s  exit \(status)\n"
        for line in String(decoding: stderr, as: UTF8.self).split(separator: "\n") where line.hasPrefix("trace: ") {
            text += "    \(line.dropFirst("trace: ".count))\n"
        }
        guard let data = text.data(using: .utf8) else { return }
        if let handle = try? FileHandle(forWritingTo: url) {
            handle.seekToEndOfFile()
            handle.write(data)
            try? handle.close()
        } else {
            try? data.write(to: url)
        }
    }

    static func decode<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(type, from: data)
    }
}

// MARK: - The JSON the CLI produces. Unknown fields are ignored, so the CLI can
// add to its output without breaking the app.

struct Status: Decodable {
    let disk: Disk?
    let schedule: Schedule
    let findings: Findings?
    /// Checked by the bundled CLI at the moment of asking, so it is this app's
    /// own answer rather than a terminal's.
    let fullDiskAccess: Bool?
}

struct Disk: Decodable {
    let used: UInt64
    let free: UInt64
    let total: UInt64
}

struct Schedule: Decodable {
    let installed: Bool
    let loaded: Bool
    let schedule: String?
    let lastRun: String?
}

struct Findings: Decodable {
    let at: UInt64
    let reclaimable: [Estimate]
    let detections: [Detection]
    let fullDiskAccess: Bool?
    let skippedProtected: Bool?
}

struct Estimate: Decodable, Identifiable {
    var id: String { name }
    let name: String
    let title: String
    let plain: String
    let bytes: UInt64?
    let upperBound: Bool
    let blockedBy: String?
    let note: String?
    let optIn: Bool
}

struct Detection: Decodable, Identifiable {
    let id: String
    let title: String
    let bytes: UInt64
    let what: String
    let how: String
    let path: String?
}

struct CleanResult: Decodable {
    let bytes: UInt64
    let freeBefore: UInt64?
    let freeAfter: UInt64?
}

struct ReportFile: Decodable {
    let path: String
}

// MARK: - Formatting that matches the report exactly (binary units).

func formatBytes(_ bytes: UInt64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"]
    var value = Double(bytes)
    var i = 0
    while value >= 1024 && i < units.count - 1 { value /= 1024; i += 1 }
    if i == 0 { return "\(bytes) B" }
    return value < 10 ? String(format: "%.1f %@", value, units[i]) : String(format: "%.0f %@", value, units[i])
}

enum Level {
    case full, low, tight, fine, plenty
    init(freePercent p: Double) {
        self = p < 5 ? .full : p < 8 ? .low : p < 15 ? .tight : p < 25 ? .fine : .plenty
    }
    var words: String {
        switch self {
        case .full: return "Nearly full"
        case .low: return "Running low"
        case .tight: return "Getting tight"
        case .fine: return "Doing fine"
        case .plenty: return "Plenty of space"
        }
    }
}
