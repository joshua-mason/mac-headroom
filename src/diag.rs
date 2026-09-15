//! The diagnostic that matters: is the missing space files, or is it pinned by
//! APFS snapshots? Snapshot-pinned blocks are invisible to `du`, and don't count
//! toward "Volume Used" either. The tell is that used and free stop moving
//! together between runs.

use crate::util::{ago, human, now, signed, state_dir, stdout_of};
use serde::Serialize;
use std::fs;
use std::io::Write;

const GB: i64 = 1 << 30;

pub struct Disk {
    pub used: u64,
    pub free: u64,
    pub total: u64,
}

fn bytes_field(diskutil_out: &str, label: &str) -> Option<u64> {
    // Lines look like: "   Volume Used Space:   174.8 GB (174847209472 Bytes) (exactly ...)"
    let line = diskutil_out.lines().find(|l| l.contains(label))?;
    let start = line.find('(')? + 1;
    let end = line[start..].find(" Bytes")? + start;
    line[start..end].trim().parse().ok()
}

pub fn disk() -> Option<Disk> {
    let info = stdout_of("diskutil", &["info", "/System/Volumes/Data"]);
    Some(Disk {
        used: bytes_field(&info, "Volume Used Space")?,
        free: bytes_field(&info, "Container Free Space")?,
        total: bytes_field(&info, "Container Total Space")?,
    })
}

#[derive(Serialize)]
pub struct Delta {
    pub previous_at: u64,
    pub used: i64,
    pub free: i64,
    /// Free space lost that file growth does not explain. Positive means
    /// something other than files (snapshots, purgeable) is holding space.
    pub unaccounted: i64,
}

#[derive(Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Assessment {
    FirstRun,
    Stable,
    FilesGrew,
    SpacePinned,
}

#[derive(Serialize)]
pub struct Report {
    pub used: u64,
    pub free: u64,
    /// total - used - free: system volume, VM, preboot, purgeable.
    pub other: u64,
    pub total: u64,
    pub snapshots: Vec<String>,
    pub update_snapshot_pinned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since_last: Option<Delta>,
    pub assessment: Assessment,
    pub advice: String,
}

/// Returns the previous (timestamp, used, free) reading, then appends the current one.
fn record(d: &Disk) -> Option<(u64, u64, u64)> {
    let path = state_dir().join("history.tsv");
    let previous = fs::read_to_string(&path).ok().and_then(|s| {
        let last = s.lines().rev().find(|l| !l.trim().is_empty())?;
        let mut f = last.split('\t');
        Some((
            f.next()?.parse().ok()?,
            f.next()?.parse().ok()?,
            f.next()?.parse().ok()?,
        ))
    });
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{}\t{}\t{}", now(), d.used, d.free);
    }
    previous
}

pub fn report() -> Option<Report> {
    let d = disk()?;
    let snaps = stdout_of("tmutil", &["listlocalsnapshots", "/"]);
    let snapshots: Vec<String> = snaps
        .lines()
        .filter(|l| l.starts_with("com.apple"))
        .map(str::to_string)
        .collect();
    let update_snapshot_pinned = snapshots.iter().any(|n| n.contains("com.apple.os.update"));

    let since_last = record(&d).map(|(previous_at, pu, pf)| {
        let used = d.used as i64 - pu as i64;
        let free = d.free as i64 - pf as i64;
        Delta {
            previous_at,
            used,
            free,
            unaccounted: -free - used,
        }
    });

    let (assessment, advice) = match &since_last {
        _ if update_snapshot_pinned => (
            Assessment::SpacePinned,
            "A staged macOS update snapshot is pinning space. Install the update and reboot to release it. Do not hunt for files first.".to_string(),
        ),
        None => (
            Assessment::FirstRun,
            "First reading recorded. Run again later to see whether used and free move together.".to_string(),
        ),
        Some(delta) if delta.unaccounted > 2 * GB => (
            Assessment::SpacePinned,
            format!(
                "{} of lost free space is not explained by file growth. That is usually snapshot-pinned or purgeable space, and du will not find it.",
                human(delta.unaccounted as u64)
            ),
        ),
        Some(delta) if delta.used > 5 * GB => (
            Assessment::FilesGrew,
            "Real files grew. Run `mac-headroom growth` to find the directory, rather than clearing caches.".to_string(),
        ),
        Some(_) => (Assessment::Stable, "Used and free moved together. Nothing hidden.".to_string()),
    };

    Some(Report {
        used: d.used,
        free: d.free,
        other: d.total.saturating_sub(d.used + d.free),
        total: d.total,
        snapshots,
        update_snapshot_pinned,
        since_last,
        assessment,
        advice,
    })
}

pub fn print_text(r: &Report) {
    println!("Data volume");
    println!("  used   {:>9}", human(r.used));
    println!("  free   {:>9}", human(r.free));
    println!(
        "  other  {:>9}   (system volume, VM, preboot, purgeable)",
        human(r.other)
    );
    println!("  total  {:>9}", human(r.total));
    println!();
    if r.snapshots.is_empty() {
        println!("Snapshots: none");
    } else {
        println!("Snapshots: {}", r.snapshots.len());
        for n in &r.snapshots {
            println!("  {n}");
        }
    }
    println!();
    if let Some(d) = &r.since_last {
        println!(
            "Since last run ({}): used {}, free {}",
            ago(d.previous_at),
            signed(d.used),
            signed(d.free)
        );
    }
    println!("{}", r.advice);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_diskutil_bytes() {
        let out = "   Volume Used Space:         174.8 GB (174847209472 Bytes) (exactly 341498456 512-Byte-Units)\n   Container Free Space:      36.3 GB (36304953344 Bytes) (exactly 70908112 512-Byte-Units)\n";
        assert_eq!(bytes_field(out, "Volume Used Space"), Some(174_847_209_472));
        assert_eq!(
            bytes_field(out, "Container Free Space"),
            Some(36_304_953_344)
        );
        assert_eq!(bytes_field(out, "Nope"), None);
    }
}
