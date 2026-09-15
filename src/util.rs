use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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
