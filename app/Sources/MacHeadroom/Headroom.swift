import Foundation

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

    /// Run a command with --json and --no-open, off the main thread.
    static func run(_ args: [String]) async throws -> Data {
        try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = executable
                process.arguments = ["--json", "--no-open"] + args
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
                DispatchQueue.global().async { stderr = err.fileHandleForReading.readDataToEndOfFile(); group.leave() }
                group.wait()
                process.waitUntilExit()
                if process.terminationStatus == 0 {
                    continuation.resume(returning: stdout)
                } else {
                    let text = String(data: stderr, encoding: .utf8)?
                        .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
                    continuation.resume(throwing: Failure(
                        message: text.isEmpty ? "mac-headroom exited with status \(process.terminationStatus)" : text))
                }
            }
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
