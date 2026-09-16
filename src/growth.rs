//! Which directory grew? Walk a root once, accumulate physical size into every
//! ancestor up to a depth, save the table, and diff against the last scan.
//! Deliberately no heuristics about what is "stale": the tool measures, the
//! reader decides.

use crate::util::{ago, human, now, signed, state_dir, tilde};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

pub type Sizes = BTreeMap<PathBuf, u64>;

/// Walk a root, accumulating physical size into every ancestor up to `depth`.
/// Returns the sizes and a count of entries that could not be read, which is
/// how an unprivileged scan of a system directory quietly undercounts.
pub fn scan(root: &Path, depth: usize) -> (Sizes, u64) {
    let mut sizes = Sizes::new();
    let mut skipped = 0u64;
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .same_file_system(true)
        .into_iter();
    for entry in walker {
        let Ok(entry) = entry else {
            skipped += 1;
            continue;
        };
        let Ok(md) = entry.metadata() else {
            skipped += 1;
            continue;
        };
        if !md.is_file() {
            continue;
        }
        let bytes = md.blocks() * 512;
        let rel = entry.path().strip_prefix(root).unwrap_or(entry.path());
        let mut acc = root.to_path_buf();
        *sizes.entry(acc.clone()).or_default() += bytes;
        for comp in rel.components().take(depth) {
            acc.push(comp);
            *sizes.entry(acc.clone()).or_default() += bytes;
        }
    }
    (sizes, skipped)
}

fn slug(root: &Path) -> String {
    root.to_string_lossy().replace('/', "_")
}

fn scans_dir() -> PathBuf {
    let dir = state_dir().join("growth");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Scans are keyed by root and depth: a diff only makes sense between scans
/// that recorded the same set of paths.
fn scan_prefix(root: &Path, depth: usize) -> String {
    format!("{}.d{depth}.", slug(root))
}

fn save(root: &Path, depth: usize, sizes: &Sizes, skipped: u64, at: u64) -> std::io::Result<()> {
    let body: String = std::iter::once(format!("#skipped\t{skipped}\n"))
        .chain(sizes.iter().map(|(p, b)| format!("{b}\t{}\n", p.display())))
        .collect();
    fs::write(
        scans_dir().join(format!("{}{at}.tsv", scan_prefix(root, depth))),
        body,
    )
}

/// Saved scans for this root and depth, newest first: (timestamp, file).
fn saved_scans(root: &Path, depth: usize) -> Vec<(u64, PathBuf)> {
    let prefix = scan_prefix(root, depth);
    let mut found: Vec<(u64, PathBuf)> = fs::read_dir(scans_dir())
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let ts = name
                        .strip_prefix(&prefix)?
                        .strip_suffix(".tsv")?
                        .parse()
                        .ok()?;
                    Some((ts, e.path()))
                })
                .collect()
        })
        .unwrap_or_default();
    found.sort_by_key(|(ts, _)| std::cmp::Reverse(*ts));
    found
}

fn read_scan(path: &Path) -> Option<(Sizes, u64)> {
    let text = fs::read_to_string(path).ok()?;
    let mut skipped = 0;
    let sizes = text
        .lines()
        .filter_map(|l| {
            let (b, p) = l.split_once('\t')?;
            if b == "#skipped" {
                skipped = p.parse().unwrap_or(0);
                return None;
            }
            Some((PathBuf::from(p), b.parse().ok()?))
        })
        .collect();
    Some((sizes, skipped))
}

/// Most recent previous scan for this root and depth: (timestamp, sizes).
fn load_previous(root: &Path, depth: usize) -> Option<(u64, Sizes)> {
    let (ts, path) = saved_scans(root, depth).into_iter().next()?;
    Some((ts, read_scan(&path)?.0))
}

#[derive(Serialize)]
pub struct Entry {
    pub path: PathBuf,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<i64>,
}

#[derive(Serialize)]
pub struct Report {
    pub root: PathBuf,
    pub depth: usize,
    pub scanned_at: u64,
    pub total: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_total: Option<u64>,
    pub min_bytes: u64,
    /// Entries the scan could not read. Above zero, every figure is a floor.
    pub skipped: u64,
    /// With a previous scan: entries whose |delta| >= min_bytes, largest change first.
    /// Without one: the largest entries by size.
    pub entries: Vec<Entry>,
}

/// How to pick and order the entries a report carries.
#[derive(Clone, Copy, PartialEq)]
pub enum Rank {
    /// Biggest now. Always meaningful, with or without a previous scan.
    Size,
    /// Biggest change since the previous scan, when there is one.
    Change,
}

pub fn report(root: &Path, depth: usize, min_bytes: u64, top: usize) -> Report {
    let scanned_at = now();
    let (sizes, skipped) = scan(root, depth);
    let previous = load_previous(root, depth);
    if let Err(e) = save(root, depth, &sizes, skipped, scanned_at) {
        eprintln!("warning: could not save scan: {e}");
    }
    let rank = if previous.is_some() {
        Rank::Change
    } else {
        Rank::Size
    };
    build(
        root, depth, scanned_at, sizes, skipped, previous, min_bytes, top, rank,
    )
}

/// The same report, from the two most recent saved scans, without rescanning.
/// None if nothing has been scanned yet.
pub fn from_saved(
    root: &Path,
    depth: usize,
    min_bytes: u64,
    top: usize,
    rank: Rank,
) -> Option<Report> {
    let mut scans = saved_scans(root, depth).into_iter();
    let (latest_at, latest_path) = scans.next()?;
    let (latest, skipped) = read_scan(&latest_path)?;
    let previous = scans
        .next()
        .and_then(|(ts, p)| Some((ts, read_scan(&p)?.0)));
    Some(build(
        root, depth, latest_at, latest, skipped, previous, min_bytes, top, rank,
    ))
}

#[allow(clippy::too_many_arguments)]
fn build(
    root: &Path,
    depth: usize,
    scanned_at: u64,
    sizes: Sizes,
    skipped: u64,
    previous: Option<(u64, Sizes)>,
    min_bytes: u64,
    top: usize,
    rank: Rank,
) -> Report {
    let total = sizes.get(root).copied().unwrap_or(0);

    let mut entries: Vec<Entry> = match &previous {
        None => sizes
            .iter()
            .filter(|(p, _)| p.as_path() != root)
            .map(|(p, &b)| Entry {
                path: p.clone(),
                bytes: b,
                previous: None,
                delta: None,
            })
            .collect(),
        Some((_, prev)) => {
            let mut all: Vec<Entry> = sizes
                .iter()
                .filter(|(p, _)| p.as_path() != root)
                .map(|(p, &b)| {
                    let previous = prev.get(p).copied();
                    let delta = b as i64 - previous.unwrap_or(0) as i64;
                    Entry {
                        path: p.clone(),
                        bytes: b,
                        previous,
                        delta: Some(delta),
                    }
                })
                .collect();
            // Paths that existed last time and are gone now.
            all.extend(
                prev.iter()
                    .filter(|(p, _)| p.as_path() != root && !sizes.contains_key(*p))
                    .map(|(p, &b)| Entry {
                        path: p.clone(),
                        bytes: 0,
                        previous: Some(b),
                        delta: Some(-(b as i64)),
                    }),
            );
            if rank == Rank::Change {
                all.retain(|e| e.delta.unwrap_or(0).unsigned_abs() >= min_bytes);
            } else {
                all.retain(|e| e.bytes >= min_bytes);
            }
            all
        }
    };

    match rank {
        Rank::Size => entries.sort_by_key(|e| std::cmp::Reverse(e.bytes)),
        Rank::Change => {
            entries.sort_by_key(|e| std::cmp::Reverse(e.delta.unwrap_or(0).unsigned_abs()))
        }
    }
    entries.truncate(top);

    Report {
        root: root.to_path_buf(),
        depth,
        scanned_at,
        total,
        previous_at: previous.as_ref().map(|(t, _)| *t),
        previous_total: previous.as_ref().and_then(|(_, s)| s.get(root).copied()),
        min_bytes,
        skipped,
        entries,
    }
}

pub fn print_text(r: &Report) {
    println!("{}  {} (depth {})", tilde(&r.root), human(r.total), r.depth);
    if r.skipped > 0 {
        println!(
            "  {} entries could not be read, so these figures are a floor. Re-run with sudo for the full picture.",
            r.skipped
        );
    }
    match (r.previous_at, r.previous_total) {
        (Some(at), Some(prev_total)) => {
            println!(
                "Compared with scan from {} ({}): {} overall. Changes of {} or more:",
                ago(at),
                human(prev_total),
                signed(r.total as i64 - prev_total as i64),
                human(r.min_bytes)
            );
            println!();
            if r.entries.is_empty() {
                println!("  nothing changed by that much");
            }
            for e in &r.entries {
                let note = match e.previous {
                    None => "  (new)",
                    Some(_) if e.bytes == 0 => "  (gone)",
                    Some(_) => "",
                };
                println!(
                    "  {:>9}  {:>9}  {}{note}",
                    signed(e.delta.unwrap_or(0)),
                    human(e.bytes),
                    tilde(&e.path)
                );
            }
        }
        _ => {
            println!("First scan saved. Largest entries now; run again later for a diff.");
            println!();
            for e in &r.entries {
                println!("  {:>9}  {}", human(e.bytes), tilde(&e.path));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_accumulates_into_ancestors() {
        let root = std::env::temp_dir().join(format!("mac-headroom-test-{}", now()));
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("a/b/f1"), vec![0u8; 8192]).unwrap();
        fs::write(root.join("a/f2"), vec![0u8; 4096]).unwrap();
        let (sizes, _) = scan(&root, 2);
        let a = sizes[&root.join("a")];
        let ab = sizes[&root.join("a/b")];
        assert_eq!(sizes[&root], a);
        assert!(a > ab && ab > 0);
        // Depth 2 stops at a/b: no entry for the file inside it.
        assert!(!sizes.contains_key(&root.join("a/b/f1")));
        fs::remove_dir_all(&root).unwrap();
    }
}
