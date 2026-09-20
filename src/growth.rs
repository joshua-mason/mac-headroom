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

/// What one walk found: sizes, how many entries could not be read, and which
/// folders those were. The folders matter when two scans are compared. A
/// folder this scan could not enter is missing from `sizes` exactly as a
/// deleted one would be, and only this list tells them apart.
pub struct Scan {
    pub sizes: Sizes,
    pub skipped: u64,
    pub unreadable: Vec<PathBuf>,
}

/// Walk a root, accumulating physical size into every ancestor up to `depth`.
/// The count of entries that could not be read is how an unprivileged scan of
/// a system directory quietly undercounts.
pub fn scan(root: &Path, depth: usize) -> Scan {
    let mut sizes = Sizes::new();
    let mut skipped = 0u64;
    // Folders left out on purpose, so that no privacy prompt appears. The
    // filter closure cannot borrow a plain Vec while the loop below runs.
    let protected = std::cell::RefCell::new(Vec::new());
    let mut unreadable: Vec<PathBuf> = Vec::new();
    let walker = walkdir::WalkDir::new(root)
        .follow_links(false)
        .same_file_system(true)
        .into_iter()
        .filter_entry(|e| {
            let skip = e.file_type().is_dir() && crate::util::skip_protected(e.path());
            if skip {
                protected.borrow_mut().push(e.path().to_path_buf());
            }
            !skip
        });
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                skipped += 1;
                if let Some(p) = err.path() {
                    unreadable.push(p.to_path_buf());
                }
                continue;
            }
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
    unreadable.extend(protected.into_inner());
    unreadable.sort();
    unreadable.dedup();
    Scan {
        sizes,
        skipped,
        unreadable,
    }
}

/// Whether what a scan could not read accounts for `path` being absent from
/// it: the path is inside an unreadable folder, or is the parent of one and
/// so had nothing left to count.
fn hidden_by(unreadable: &[PathBuf], path: &Path) -> bool {
    unreadable
        .iter()
        .any(|u| path.starts_with(u) || u.starts_with(path))
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

fn save(root: &Path, depth: usize, scan: &Scan, at: u64) -> std::io::Result<()> {
    let body: String = std::iter::once(format!("#skipped\t{}\n", scan.skipped))
        .chain(
            scan.unreadable
                .iter()
                .map(|p| format!("#unreadable\t{}\n", p.display())),
        )
        .chain(
            scan.sizes
                .iter()
                .map(|(p, b)| format!("{b}\t{}\n", p.display())),
        )
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

fn read_scan(path: &Path) -> Option<Scan> {
    Some(parse_scan(&fs::read_to_string(path).ok()?))
}

/// A scan saved before unreadable folders were recorded has none listed, and
/// is compared the old way: there is nothing to go on.
fn parse_scan(text: &str) -> Scan {
    let mut skipped = 0;
    let mut unreadable = Vec::new();
    let sizes = text
        .lines()
        .filter_map(|l| {
            let (b, p) = l.split_once('\t')?;
            match b {
                "#skipped" => {
                    skipped = p.parse().unwrap_or(0);
                    None
                }
                "#unreadable" => {
                    unreadable.push(PathBuf::from(p));
                    None
                }
                _ => Some((PathBuf::from(p), b.parse().ok()?)),
            }
        })
        .collect();
    Scan {
        sizes,
        skipped,
        unreadable,
    }
}

/// Most recent previous scan for this root and depth, with its timestamp.
fn load_previous(root: &Path, depth: usize) -> Option<(u64, Scan)> {
    let (ts, path) = saved_scans(root, depth).into_iter().next()?;
    Some((ts, read_scan(&path)?))
}

/// Why an entry has no figure to compare: one of the two scans could not
/// read it. It was not created and it was not deleted.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// Measured last time, unreadable this time. Still there, as far as
    /// anyone knows.
    UnreadableNow,
    /// Unreadable last time, measured this time. Not new.
    UnreadableBefore,
}

#[derive(Serialize)]
pub struct Entry {
    pub path: PathBuf,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<i64>,
    /// Set when the two scans could not both read this entry. There is then
    /// no `delta`, because a change in permission is not a change in size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access: Option<Access>,
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
    /// The two scans could read different folders, usually because one had
    /// Full Disk Access and the other did not. The totals, and the change in
    /// any folder above an affected one, are then not like for like.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub access_differs: bool,
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
    let latest = scan(root, depth);
    let previous = load_previous(root, depth);
    if let Err(e) = save(root, depth, &latest, scanned_at) {
        eprintln!("warning: could not save scan: {e}");
    }
    let rank = if previous.is_some() {
        Rank::Change
    } else {
        Rank::Size
    };
    build(
        root, depth, scanned_at, latest, previous, min_bytes, top, rank,
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
    let latest = read_scan(&latest_path)?;
    let previous = scans.next().and_then(|(ts, p)| Some((ts, read_scan(&p)?)));
    Some(build(
        root, depth, latest_at, latest, previous, min_bytes, top, rank,
    ))
}

#[allow(clippy::too_many_arguments)]
fn build(
    root: &Path,
    depth: usize,
    scanned_at: u64,
    latest: Scan,
    previous: Option<(u64, Scan)>,
    min_bytes: u64,
    top: usize,
    rank: Rank,
) -> Report {
    let Scan {
        sizes,
        skipped,
        unreadable,
    } = latest;
    let total = sizes.get(root).copied().unwrap_or(0);
    let access_differs = previous
        .as_ref()
        .is_some_and(|(_, p)| p.unreadable != unreadable);

    let mut entries: Vec<Entry> = match &previous {
        None => sizes
            .iter()
            .filter(|(p, _)| p.as_path() != root)
            .map(|(p, &b)| Entry {
                path: p.clone(),
                bytes: b,
                previous: None,
                delta: None,
                access: None,
            })
            .collect(),
        Some((_, prev)) => {
            let mut all: Vec<Entry> = sizes
                .iter()
                .filter(|(p, _)| p.as_path() != root)
                .map(|(p, &b)| {
                    let previous = prev.sizes.get(p).copied();
                    // Absent last time because it could not be read is not
                    // the same as absent because it did not exist.
                    if previous.is_none() && hidden_by(&prev.unreadable, p) {
                        return Entry {
                            path: p.clone(),
                            bytes: b,
                            previous: None,
                            delta: None,
                            access: Some(Access::UnreadableBefore),
                        };
                    }
                    Entry {
                        path: p.clone(),
                        bytes: b,
                        previous,
                        delta: Some(b as i64 - previous.unwrap_or(0) as i64),
                        access: None,
                    }
                })
                .collect();
            // Paths measured last time and absent now: gone, unless this
            // scan could not read them, in which case nobody knows.
            all.extend(
                prev.sizes
                    .iter()
                    .filter(|(p, _)| p.as_path() != root && !sizes.contains_key(*p))
                    .map(|(p, &b)| {
                        let hidden = hidden_by(&unreadable, p);
                        Entry {
                            path: p.clone(),
                            bytes: 0,
                            previous: Some(b),
                            delta: (!hidden).then_some(-(b as i64)),
                            access: hidden.then_some(Access::UnreadableNow),
                        }
                    }),
            );
            // An entry only one scan could read has no change to rank by. It
            // is kept by the size that was measured, so that a folder which
            // stopped being readable is said out loud and not dropped.
            all.retain(|e| weight(rank, e) >= min_bytes);
            all
        }
    };

    entries.sort_by_key(|e| std::cmp::Reverse(weight(rank, e)));
    entries.truncate(top);

    Report {
        root: root.to_path_buf(),
        depth,
        scanned_at,
        total,
        previous_at: previous.as_ref().map(|(t, _)| *t),
        previous_total: previous
            .as_ref()
            .and_then(|(_, s)| s.sizes.get(root).copied()),
        min_bytes,
        skipped,
        access_differs,
        entries,
    }
}

/// What an entry is ranked and filtered by.
fn weight(rank: Rank, e: &Entry) -> u64 {
    match (rank, e.access) {
        (_, Some(_)) => e.bytes.max(e.previous.unwrap_or(0)),
        (Rank::Change, None) => e.delta.unwrap_or(0).unsigned_abs(),
        (Rank::Size, None) => e.bytes,
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
            if r.access_differs {
                println!("The two scans could not read the same folders, usually because one had Full Disk Access and the other did not. The overall figure, and the change in any folder above one marked below, is not like for like.");
            }
            println!();
            if r.entries.is_empty() {
                println!("  nothing changed by that much");
            }
            for e in &r.entries {
                let note = match (e.access, e.previous) {
                    (Some(Access::UnreadableNow), Some(was)) => {
                        format!("  (could not be read this time, was {})", human(was))
                    }
                    (Some(_), _) => "  (could not be read last time)".to_string(),
                    (None, None) => "  (new)".to_string(),
                    (None, Some(_)) if e.bytes == 0 => "  (gone)".to_string(),
                    (None, Some(_)) => String::new(),
                };
                let change = e.delta.map_or_else(|| "?".to_string(), signed);
                println!(
                    "  {:>9}  {:>9}  {}{note}",
                    change,
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

    fn saved(text: &str) -> Scan {
        parse_scan(text)
    }

    fn entry<'a>(r: &'a Report, path: &str) -> &'a Entry {
        r.entries
            .iter()
            .find(|e| e.path == Path::new(path))
            .unwrap_or_else(|| panic!("no entry for {path}"))
    }

    /// A scan with Full Disk Access followed by one without. The folders the
    /// second could not read were reported as gone, and they were never
    /// deleted.
    #[test]
    fn a_folder_that_became_unreadable_is_not_gone() {
        let before =
            saved("#skipped\t0\n900\t/h\n500\t/h/Music\n500\t/h/Music/GarageBand\n400\t/h/old\n");
        let after = saved("#skipped\t1\n#unreadable\t/h/Music/GarageBand\n0\t/h\n");
        let r = build(
            Path::new("/h"),
            2,
            2,
            after,
            Some((1, before)),
            100,
            10,
            Rank::Change,
        );

        assert!(r.access_differs);
        let hidden = entry(&r, "/h/Music/GarageBand");
        assert_eq!(hidden.access, Some(Access::UnreadableNow));
        assert_eq!(hidden.delta, None);
        assert_eq!(hidden.previous, Some(500));
        // Its parent held nothing else, so it vanished for the same reason.
        assert_eq!(entry(&r, "/h/Music").access, Some(Access::UnreadableNow));
        // A folder nothing explains really is gone.
        let gone = entry(&r, "/h/old");
        assert_eq!((gone.access, gone.delta), (None, Some(-400)));
    }

    #[test]
    fn a_folder_that_became_readable_is_not_new() {
        let before = saved("#skipped\t1\n#unreadable\t/h/Music\n100\t/h\n");
        let after = saved("#skipped\t0\n900\t/h\n500\t/h/Music\n300\t/h/fresh\n");
        let r = build(
            Path::new("/h"),
            2,
            2,
            after,
            Some((1, before)),
            100,
            10,
            Rank::Change,
        );

        let seen = entry(&r, "/h/Music");
        assert_eq!(
            (seen.access, seen.delta),
            (Some(Access::UnreadableBefore), None)
        );
        let fresh = entry(&r, "/h/fresh");
        assert_eq!((fresh.access, fresh.delta), (None, Some(300)));
    }

    /// Scans saved by older versions list no unreadable folders. They compare
    /// as they always did, and say nothing about access.
    #[test]
    fn scans_without_the_record_compare_the_old_way() {
        let before = saved("#skipped\t0\n500\t/h\n500\t/h/a\n");
        let after = saved("#skipped\t0\n0\t/h\n");
        let r = build(
            Path::new("/h"),
            2,
            2,
            after,
            Some((1, before)),
            100,
            10,
            Rank::Change,
        );
        assert!(!r.access_differs);
        assert_eq!(entry(&r, "/h/a").delta, Some(-500));
    }

    #[test]
    fn scan_accumulates_into_ancestors() {
        let root = std::env::temp_dir().join(format!("mac-headroom-test-{}", now()));
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("a/b/f1"), vec![0u8; 8192]).unwrap();
        fs::write(root.join("a/f2"), vec![0u8; 4096]).unwrap();
        let Scan { sizes, .. } = scan(&root, 2);
        let a = sizes[&root.join("a")];
        let ab = sizes[&root.join("a/b")];
        assert_eq!(sizes[&root], a);
        assert!(a > ab && ab > 0);
        // Depth 2 stops at a/b: no entry for the file inside it.
        assert!(!sizes.contains_key(&root.join("a/b/f1")));
        fs::remove_dir_all(&root).unwrap();
    }
}
