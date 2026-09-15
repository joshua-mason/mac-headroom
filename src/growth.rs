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

pub fn scan(root: &Path, depth: usize) -> Sizes {
    let mut sizes = Sizes::new();
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .same_file_system(true)
        .into_iter()
        .filter_map(Result::ok);
    for entry in walker {
        let Ok(md) = entry.metadata() else { continue };
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
    sizes
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

fn save(root: &Path, depth: usize, sizes: &Sizes, at: u64) -> std::io::Result<()> {
    let body: String = sizes
        .iter()
        .map(|(p, b)| format!("{b}\t{}\n", p.display()))
        .collect();
    fs::write(
        scans_dir().join(format!("{}{at}.tsv", scan_prefix(root, depth))),
        body,
    )
}

/// Most recent previous scan for this root and depth: (timestamp, sizes).
fn load_previous(root: &Path, depth: usize) -> Option<(u64, Sizes)> {
    let prefix = scan_prefix(root, depth);
    let mut latest: Option<(u64, PathBuf)> = None;
    for e in fs::read_dir(scans_dir()).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix(&prefix) else {
            continue;
        };
        let Some(ts) = rest
            .strip_suffix(".tsv")
            .and_then(|s| s.parse::<u64>().ok())
        else {
            continue;
        };
        if latest.as_ref().is_none_or(|(t, _)| ts > *t) {
            latest = Some((ts, e.path()));
        }
    }
    let (ts, path) = latest?;
    let sizes = fs::read_to_string(path)
        .ok()?
        .lines()
        .filter_map(|l| {
            let (b, p) = l.split_once('\t')?;
            Some((PathBuf::from(p), b.parse().ok()?))
        })
        .collect();
    Some((ts, sizes))
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
    /// With a previous scan: entries whose |delta| >= min_bytes, largest change first.
    /// Without one: the largest entries by size.
    pub entries: Vec<Entry>,
}

pub fn report(root: &Path, depth: usize, min_bytes: u64, top: usize) -> Report {
    let scanned_at = now();
    let sizes = scan(root, depth);
    let total = sizes.get(root).copied().unwrap_or(0);
    let previous = load_previous(root, depth);
    if let Err(e) = save(root, depth, &sizes, scanned_at) {
        eprintln!("warning: could not save scan: {e}");
    }

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
            all.retain(|e| e.delta.unwrap_or(0).unsigned_abs() >= min_bytes);
            all
        }
    };

    match previous {
        None => entries.sort_by_key(|e| std::cmp::Reverse(e.bytes)),
        Some(_) => entries.sort_by_key(|e| std::cmp::Reverse(e.delta.unwrap_or(0).unsigned_abs())),
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
        entries,
    }
}

pub fn print_text(r: &Report) {
    println!("{}  {} (depth {})", tilde(&r.root), human(r.total), r.depth);
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
        let sizes = scan(&root, 2);
        let a = sizes[&root.join("a")];
        let ab = sizes[&root.join("a/b")];
        assert_eq!(sizes[&root], a);
        assert!(a > ab && ab > 0);
        // Depth 2 stops at a/b: no entry for the file inside it.
        assert!(!sizes.contains_key(&root.join("a/b/f1")));
        fs::remove_dir_all(&root).unwrap();
    }
}
