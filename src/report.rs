//! A single self-contained HTML file with everything `status`, `diagnose`
//! history, the last `growth` diff, the cleaner table and the audit trail
//! show, drawn as charts and tables. Read-only: it reads saved state and
//! never scans or deletes. No server, no network, no external assets.

use crate::util::{home, now, state_dir, stdout_of};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct Reading {
    at: u64,
    used: u64,
    free: u64,
}

#[derive(Serialize)]
struct AuditEntry {
    at: u64,
    cleaner: String,
    result: String,
    /// None when nothing measured it. Shown as unknown, never as zero.
    bytes: Option<u64>,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

/// One audit line. The first format had five columns and wrote 0 for every
/// command it ran without measuring, so for those old lines a command's 0 means
/// unknown. The current format adds a sixth column and writes "-" for unknown.
fn parse_audit_line(line: &str) -> Option<AuditEntry> {
    let f: Vec<&str> = line.splitn(6, '\t').collect();
    if f.len() < 5 {
        return None;
    }
    let result = f[2].to_string();
    let is_command = matches!(result.as_str(), "ran" | "failed" | "skipped" | "would_run")
        && !f[4].starts_with('/');
    let bytes = match (f.len(), f[3]) {
        (_, "-") => None,
        (5, _) if is_command => None,
        (_, n) => n.parse().ok(),
    };
    let detail = f
        .get(5)
        .map(|d| d.trim())
        .filter(|d| !d.is_empty())
        .map(str::to_string);
    Some(AuditEntry {
        at: f[0].parse().ok()?,
        cleaner: f[1].to_string(),
        result,
        bytes,
        path: f[4].to_string(),
        detail,
    })
}

#[derive(Serialize)]
pub struct Data {
    generated_at: u64,
    host: String,
    version: &'static str,
    repository: &'static str,
    home: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    disk: Option<DiskNow>,
    snapshots: Vec<String>,
    volumes: crate::volumes::Volumes,
    update_snapshot_pinned: bool,
    history: Vec<Reading>,
    #[serde(skip_serializing_if = "Option::is_none")]
    growth: Option<crate::growth::Report>,
    /// One per extra root in the config, so the page can show the part of the
    /// data volume that is not in the home folder.
    growth_roots: Vec<crate::growth::Report>,
    config: crate::config::Check,
    cleaners: Vec<crate::cleaners::Cleaner>,
    schedule: crate::schedule::Status,
    audit: Vec<AuditEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_run_log: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    findings: Option<crate::findings::Findings>,
    /// Each completed weekly routine: when, and free space either side.
    runs: Vec<Run>,
}

#[derive(Serialize)]
struct DiskNow {
    used: u64,
    free: u64,
    total: u64,
    other: u64,
    /// The whole drive, when it could be measured. The page uses it to account
    /// for the difference between the capacity it reports and the size on the
    /// box, which is otherwise the page's most suspicious-looking number.
    #[serde(skip_serializing_if = "Option::is_none")]
    media: Option<u64>,
}

#[derive(Serialize)]
struct Run {
    at: u64,
    free_before: u64,
    free_after: u64,
}

fn runs() -> Vec<Run> {
    fs::read_to_string(state_dir().join("runs.tsv"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            Some(Run {
                at: f.next()?.parse().ok()?,
                free_before: f.next()?.parse().ok()?,
                free_after: f.next()?.parse().ok()?,
            })
        })
        .collect()
}

fn history() -> Vec<Reading> {
    fs::read_to_string(state_dir().join("history.tsv"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            Some(Reading {
                at: f.next()?.parse().ok()?,
                used: f.next()?.parse().ok()?,
                free: f.next()?.parse().ok()?,
            })
        })
        .collect()
}

fn audit(limit: usize) -> Vec<AuditEntry> {
    let text = fs::read_to_string(state_dir().join("audit.log")).unwrap_or_default();
    let mut entries: Vec<AuditEntry> = text.lines().filter_map(parse_audit_line).collect();
    entries.reverse();
    entries.truncate(limit);
    entries
}

fn last_run_log() -> Option<String> {
    let text = fs::read_to_string(crate::schedule::log_path()).ok()?;
    let start = text.rfind("=== mac-headroom scheduled run ===")?;
    let line_start = text[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    Some(text[line_start..].trim_end().to_string())
}

fn hostname() -> String {
    let name = stdout_of("scutil", &["--get", "ComputerName"]);
    let name = name.trim();
    if name.is_empty() {
        stdout_of("hostname", &["-s"]).trim().to_string()
    } else {
        name.to_string()
    }
}

/// With MAC_HEADROOM_TRACE set, time a step and say so on stderr.
fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let trace = std::env::var_os("MAC_HEADROOM_TRACE").is_some();
    let start = std::time::Instant::now();
    let out = f();
    if trace {
        eprintln!("trace: {:>6} ms  {label}", start.elapsed().as_millis());
    }
    out
}

pub fn gather() -> Data {
    let cleaners = timed("cleaners", || {
        crate::config::all_cleaners().unwrap_or_default()
    });
    let snapshots = timed("snapshots", crate::diag::snapshots);
    let update_snapshot_pinned = snapshots.iter().any(|n| n.contains("com.apple.os.update"));
    Data {
        generated_at: now(),
        host: timed("hostname", hostname),
        version: env!("CARGO_PKG_VERSION"),
        repository: crate::suggest::REPOSITORY,
        home: home().to_string_lossy().into_owned(),
        disk: timed("disk", || {
            crate::diag::disk().map(|d| DiskNow {
                used: d.used,
                free: d.free,
                total: d.total,
                other: d.total.saturating_sub(d.used + d.free),
                media: d.media,
            })
        }),
        snapshots,
        volumes: timed("volumes", crate::volumes::gather),
        update_snapshot_pinned,
        history: timed("history", history),
        // More entries than the terminal shows: the page nests them into a
        // tree, so it needs the parents as well as the leaves.
        // Ranked by size, not by change: the page renders the size view and
        // the change view from the same list, and a change-ranked list would
        // silently drop everything that did not move.
        growth: timed("growth home", || {
            crate::growth::from_saved(&home(), 3, 100_000_000, 80, crate::growth::Rank::Size)
        }),
        growth_roots: timed("growth other roots", || {
            crate::config::scan_roots()
                .iter()
                .filter_map(|r| {
                    crate::growth::from_saved(r, 3, 100_000_000, 40, crate::growth::Rank::Size)
                })
                .collect()
        }),
        config: timed("config check", crate::config::check),
        cleaners,
        schedule: timed("schedule status", crate::schedule::status),
        audit: timed("audit", || audit(500)),
        last_run_log: timed("last run log", last_run_log),
        findings: timed("findings", crate::findings::load_current),
        runs: timed("runs", runs),
    }
}

pub fn default_path() -> PathBuf {
    state_dir().join("report.html")
}

pub fn write(data: &Data, out: &Path) -> std::io::Result<()> {
    // `</` inside a <script> would end it early; JSON never needs the raw form.
    let json = serde_json::to_string(data)
        .expect("serialize report")
        .replace("</", "<\\/");
    fs::write(out, TEMPLATE.replace("__DATA__", &json))
}

pub fn open(path: &Path) -> bool {
    std::process::Command::new("open")
        .arg(path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

const TEMPLATE: &str = include_str!("report.html");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_command_lines_are_unmeasured_not_zero() {
        let e = parse_audit_line("1\tnpm-cache\tran\t0\tnpm cache clean --force").unwrap();
        assert_eq!(e.bytes, None);
        let e =
            parse_audit_line("1\tlanguage-caches\tdeleted\t0\t/Users/x/Library/Caches/a").unwrap();
        assert_eq!(e.bytes, Some(0), "an empty file really was 0 bytes");
    }

    #[test]
    fn new_lines_carry_measurement_and_reason() {
        let e = parse_audit_line("1\tnpm-cache\tran\t1048576\tnpm cache clean --force\t").unwrap();
        assert_eq!(e.bytes, Some(1_048_576));
        assert_eq!(e.detail, None);
        let e = parse_audit_line(
            "1\tnpm-cache\tfailed\t-\tnpm cache clean --force\tLibrary not loaded",
        )
        .unwrap();
        assert_eq!(e.bytes, None);
        assert_eq!(e.detail.as_deref(), Some("Library not loaded"));
    }
}
