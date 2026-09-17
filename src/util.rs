use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Whether a path lives on the volume that holds user data.
///
/// Only locations there belong in the breakdown under "Your data". Several
/// paths that look like part of the system are really firmlinked onto it
/// (`/System/Library/AssetsV2` holds gigabytes of downloaded OS assets), and
/// anything on another volume would be counted against the wrong total.
pub fn on_data_volume(p: &Path) -> bool {
    match (
        std::fs::metadata(p),
        std::fs::metadata("/System/Volumes/Data"),
    ) {
        (Ok(a), Ok(b)) => a.dev() == b.dev(),
        _ => false,
    }
}

/// Whether this process can read folders macOS keeps behind Full Disk Access.
///
/// Without it the Trash, Mail and Safari data are unreadable, so a scan quietly
/// misses what is often the single largest thing on the disk. None means there
/// was nothing to test against.
pub fn full_disk_access() -> Option<bool> {
    let h = home();
    for probe in [
        h.join(".Trash"),
        h.join("Library/Safari"),
        h.join("Library/Mail"),
    ] {
        if std::fs::symlink_metadata(&probe).is_err() {
            continue;
        }
        return match std::fs::read_dir(&probe) {
            Ok(_) => Some(true),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Some(false),
            Err(_) => continue,
        };
    }
    None
}

/// Percent-encode a value for a URL query string.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

static SKIP_PROTECTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Turn on scanning that never reads a folder macOS guards with a privacy
/// prompt. Without Full Disk Access, an app walking into each of these makes
/// macOS stop and ask the person, one folder at a time.
pub fn set_skip_protected(on: bool) {
    SKIP_PROTECTED.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn skipping_protected() -> bool {
    SKIP_PROTECTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Home-relative locations macOS protects: reading them either prompts
/// (Desktop, Documents, Downloads, Photos, other apps' data) or needs Full
/// Disk Access (Mail, Messages, Safari, the Trash and similar).
const PROTECTED: &[&str] = &[
    "Desktop",
    "Documents",
    "Downloads",
    ".Trash",
    "Library/Mail",
    "Library/Messages",
    "Library/Safari",
    "Library/Calendars",
    "Library/Reminders",
    "Library/Containers",
    "Library/Group Containers",
    "Library/Mobile Documents",
    "Library/Cookies",
    "Library/Suggestions",
    "Library/HomeKit",
    "Library/IdentityServices",
    "Library/Accounts",
    "Library/Biome",
    "Library/Metadata/CoreSpotlight",
    "Library/PersonalizationPortrait",
    "Library/Application Support/AddressBook",
    "Library/Application Support/CallHistoryDB",
    "Library/Application Support/CallHistoryTransactions",
    "Library/Application Support/MobileSync",
    "Library/Application Support/com.apple.TCC",
    "Library/Application Support/Knowledge",
];

/// Whether a path should be left alone because protected folders are being skipped.
pub fn skip_protected(path: &Path) -> bool {
    if !skipping_protected() {
        return false;
    }
    if path.extension().is_some_and(|e| e == "photoslibrary") {
        return true;
    }
    let Ok(rel) = path.strip_prefix(home()) else {
        return false;
    };
    PROTECTED.iter().any(|p| rel == Path::new(p))
}

pub fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").expect("HOME is not set"))
}

/// Where mac-headroom keeps history, growth scans and the audit log.
pub fn state_dir() -> PathBuf {
    let dir = home().join(".local/state/mac-headroom");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Expand a leading `~/` to the user's home directory.
pub fn expand(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => home().join(rest).to_string_lossy().into_owned(),
        None => p.to_string(),
    }
}

/// Shorten a path under home to `~/...` for display.
pub fn tilde(p: &Path) -> String {
    match p.strip_prefix(home()) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

/// Physical bytes on disk, summed from block counts. Logical file length would
/// overcount sparse files (Docker.raw) and APFS clones, which is exactly the
/// mistake that makes "reclaimed" numbers untrustworthy.
pub fn disk_usage(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|e| e.metadata().ok())
        .map(|md| md.blocks() * 512)
        .sum()
}

pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

pub fn signed(delta: i64) -> String {
    let sign = if delta < 0 { "-" } else { "+" };
    format!("{sign}{}", human(delta.unsigned_abs()))
}

/// "3d 4h ago" style, for timestamps in the history files.
pub fn ago(epoch: u64) -> String {
    let secs = now().saturating_sub(epoch);
    let (d, h, m) = (secs / 86_400, (secs % 86_400) / 3600, (secs % 3600) / 60);
    if d > 0 {
        format!("{d}d {h}h ago")
    } else if h > 0 {
        format!("{h}h {m}m ago")
    } else {
        format!("{m}m ago")
    }
}

/// Exact process-name match, same semantics as `pgrep -x`.
pub fn is_running(process: &str) -> bool {
    Command::new("pgrep")
        .args(["-x", process])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(bin).is_file()))
        .unwrap_or(false)
}

/// Run a command and return its stdout, or an empty string if it can't run.
pub fn stdout_of(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_folders_are_only_skipped_when_asked() {
        let docs = home().join("Documents");
        set_skip_protected(false);
        assert!(!skip_protected(&docs));
        set_skip_protected(true);
        assert!(skip_protected(&docs));
        assert!(skip_protected(&home().join("Library/Group Containers")));
        assert!(skip_protected(Path::new(
            "/Users/x/Pictures/Photos Library.photoslibrary"
        )));
        assert!(!skip_protected(&home().join("Library/Caches")));
        assert!(
            !skip_protected(&home().join("Documents/project")),
            "only the folder itself is filtered"
        );
        set_skip_protected(false);
    }

    #[test]
    fn url_encoding_keeps_only_unreserved_characters() {
        assert_eq!(url_encode("a b/~c"), "a%20b%2F~c");
        assert_eq!(url_encode("línea\n"), "l%C3%ADnea%0A");
    }

    #[test]
    fn human_units() {
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1023), "1023 B");
        assert_eq!(human(1024), "1.0 KB");
        assert_eq!(human(1_500_000_000), "1.4 GB");
    }

    #[test]
    fn signed_deltas() {
        assert_eq!(signed(-2048), "-2.0 KB");
        assert_eq!(signed(2048), "+2.0 KB");
    }
}
