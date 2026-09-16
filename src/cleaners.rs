use crate::util::{ago, disk_usage, expand, human, is_running, now, on_path, tilde};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    Builtin,
    Config,
}

/// One thing that can be cleared. Built-ins are constructed in `builtins()`;
/// users add their own in the config file with the same fields.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Cleaner {
    pub name: String,
    #[serde(default)]
    pub summary: String,
    /// The reason this is safe to delete. Every cleaner must justify itself.
    pub why_safe: String,
    /// Skip the cleaner when this process is running (exact name, as `pgrep -x`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_if_running: Option<String>,
    /// Globs under `~/`. Every match is deleted, subject to the filters below.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Alternative to `paths`: run a tool's own cache-clearing command.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command: Vec<String>,
    /// In each directory, keep the N most recently modified matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_newest: Option<usize>,
    /// Only delete a match when nothing inside it was modified in the last N days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older_than_days: Option<u64>,
    #[serde(default, skip_deserializing)]
    pub source: Source,
    #[serde(default, skip_deserializing)]
    pub disabled: bool,
    /// Left out of a plain `clean`. The caller has to name it with --only, or
    /// list it under `enable` in the config. For anything that removes what a
    /// person would recognise as their own content.
    #[serde(default, skip_deserializing)]
    pub opt_in: bool,
    /// Built-in only: files are found by asking an application's own database
    /// what it still references, rather than by matching a path.
    #[serde(skip)]
    pub orphans: Option<&'static crate::orphans::OrphanSpec>,
}

fn paths(name: &str, summary: &str, why_safe: &str, skip: Option<&str>, globs: &[&str]) -> Cleaner {
    Cleaner {
        name: name.into(),
        summary: summary.into(),
        why_safe: why_safe.into(),
        skip_if_running: skip.map(Into::into),
        paths: globs.iter().map(|s| s.to_string()).collect(),
        command: vec![],
        keep_newest: None,
        older_than_days: None,
        source: Source::Builtin,
        disabled: false,
        opt_in: false,
        orphans: None,
    }
}

fn cmd(name: &str, summary: &str, why_safe: &str, argv: &[&str]) -> Cleaner {
    Cleaner {
        command: argv.iter().map(|s| s.to_string()).collect(),
        ..paths(name, summary, why_safe, None, &[])
    }
}

fn orphan_cleaner(
    name: &str,
    summary: &str,
    why_safe: &str,
    skip: Option<&str>,
    spec: &'static crate::orphans::OrphanSpec,
) -> Cleaner {
    Cleaner {
        opt_in: true,
        orphans: Some(spec),
        ..paths(name, summary, why_safe, skip, &[])
    }
}

pub fn builtins() -> Vec<Cleaner> {
    vec![
        paths(
            "xcode-derived-data",
            "Xcode build intermediates",
            "Rebuilt on the next build. Skipped while Xcode is open so a live build is not disrupted.",
            Some("Xcode"),
            &["~/Library/Developer/Xcode/DerivedData/*"],
        ),
        paths(
            "updater-leftovers",
            "Downloaded app updates that were already applied",
            "Electron/Squirrel updaters leave the installed update on disk. The app re-downloads if it ever needs it.",
            None,
            &[
                "~/Library/Caches/com.microsoft.VSCode.ShipIt/*",
                "~/Library/Caches/com.tinyspeck.slackmacgap.ShipIt/*",
                "~/Library/Caches/electron/*",
            ],
        ),
        paths(
            "language-caches",
            "pip, SwiftPM, node-gyp and TypeScript download caches",
            "Pure download caches. The next install fetches what it needs again.",
            None,
            &[
                "~/Library/Caches/pip/*",
                "~/Library/Caches/org.swift.swiftpm/*",
                "~/Library/Caches/node-gyp/*",
                "~/Library/Caches/typescript/*",
            ],
        ),
        paths(
            "spotify-cache",
            "Spotify streaming cache",
            "Streamed audio only. Playlists and downloads for offline live elsewhere. Skipped while Spotify is open.",
            Some("Spotify"),
            &["~/Library/Caches/com.spotify.client/*"],
        ),
        paths(
            "chrome-cache",
            "Chrome HTTP, code and GPU caches (profile data untouched)",
            "Only ~/Library/Caches/Google/Chrome. Cookies, history, IndexedDB and Service Workers live under Application Support and are never touched. Skipped while Chrome is open.",
            Some("Google Chrome"),
            &[
                "~/Library/Caches/Google/Chrome/*/Cache",
                "~/Library/Caches/Google/Chrome/*/Code Cache",
                "~/Library/Caches/Google/Chrome/*/GPUCache",
            ],
        ),
        cmd(
            "npm-cache",
            "npm package cache",
            "npm's own cache clean. Packages re-download on the next install.",
            &["npm", "cache", "clean", "--force"],
        ),
        cmd(
            "pnpm-store",
            "pnpm store (unreferenced packages only)",
            "pnpm's own prune. It removes only packages no project references.",
            &["pnpm", "store", "prune"],
        ),
        cmd(
            "uv-cache",
            "uv Python package cache",
            "uv's own cache clean. Wheels re-download on the next sync.",
            &["uv", "cache", "clean"],
        ),
        cmd(
            "go-build-cache",
            "Go build cache",
            "Go's own clean. The next build recompiles.",
            &["go", "clean", "-cache"],
        ),
        orphan_cleaner(
            "whatsapp-orphans",
            "WhatsApp media it no longer has any record of",
            "Every file is checked against WhatsApp's own database, including its write-ahead log, and only files that nothing in it references are removed. Re-linking the Mac as a device replaces that database, which strands everything downloaded under the old one: the app cannot see those files, so its storage screen never frees them. Anything still on your phone or within WhatsApp's retention re-downloads when you scroll back. Skipped while WhatsApp is open, and refused outright if the database cannot be read.",
            Some("WhatsApp"),
            &crate::orphans::WHATSAPP,
        ),
        cmd(
            "homebrew",
            "Homebrew downloads and old formula versions",
            "brew cleanup with --prune=all. Keeps every installed formula, removes only downloads and superseded versions.",
            &["brew", "cleanup", "-s", "--prune=all"],
        ),
    ]
}

pub fn find<'a>(all: &'a [Cleaner], name: &str) -> Option<&'a Cleaner> {
    all.iter().find(|c| c.name == name)
}

impl Cleaner {
    /// Reject anything that could not be run safely. Used for config entries.
    pub fn validate(&self) -> Result<(), String> {
        let n = &self.name;
        if n.is_empty()
            || !n
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(format!(
                "cleaner name {n:?} must be lowercase letters, digits and dashes"
            ));
        }
        if self.why_safe.trim().is_empty() {
            return Err(format!(
                "{n}: why_safe is required. Say why deleting this is safe."
            ));
        }
        if self.orphans.is_some() {
            return Ok(());
        }
        match (self.paths.is_empty(), self.command.is_empty()) {
            (true, true) => return Err(format!("{n}: needs either paths or command")),
            (false, false) => return Err(format!("{n}: paths and command are mutually exclusive")),
            _ => {}
        }
        if self.paths.is_empty() && (self.keep_newest.is_some() || self.older_than_days.is_some()) {
            return Err(format!(
                "{n}: keep_newest and older_than_days only apply to paths"
            ));
        }
        for p in &self.paths {
            let Some(rest) = p.strip_prefix("~/") else {
                return Err(format!("{n}: path {p:?} must start with ~/"));
            };
            let comps: Vec<&str> = rest.split('/').collect();
            if comps
                .iter()
                .any(|c| c.is_empty() || *c == "." || *c == "..")
            {
                return Err(format!(
                    "{n}: path {p:?} must not contain empty, . or .. components"
                ));
            }
            if comps.len() < 2 || comps[0].contains(['*', '?', '[']) {
                return Err(format!(
                    "{n}: path {p:?} is too broad. It must name a directory under ~ and something inside it, e.g. ~/Library/Caches/foo/*"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Serialize, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    WouldClear,
    Cleared,
    WouldRun,
    Ran,
    Failed,
    Skipped,
    Nothing,
}

#[derive(Serialize)]
pub struct Target {
    pub path: PathBuf,
    pub bytes: u64,
    /// Set when a filter kept this match. It is neither deleted nor counted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kept: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct Outcome {
    pub name: String,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub targets: Vec<Target>,
    /// Bytes reclaimed, or that would be. Always 0 for command cleaners.
    pub bytes: u64,
}

fn epoch(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Most recent modification anywhere inside a path (or of the path itself).
fn newest_mtime(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .map(epoch)
        .max()
        .unwrap_or(0)
}

fn own_mtime(path: &Path) -> u64 {
    std::fs::symlink_metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(epoch)
        .unwrap_or(0)
}

/// Every glob match, paired with the reason a filter kept it (if any). Sorted.
fn candidates(c: &Cleaner) -> Vec<(PathBuf, Option<String>)> {
    let mut paths: Vec<PathBuf> = c
        .paths
        .iter()
        .flat_map(|g| glob::glob(&expand(g)).into_iter().flatten().flatten())
        .collect();
    paths.sort();
    paths.dedup();

    let mut out: Vec<(PathBuf, Option<String>)> = paths.into_iter().map(|p| (p, None)).collect();

    if let Some(days) = c.older_than_days {
        let cutoff = now().saturating_sub(days * 86_400);
        for (p, kept) in out.iter_mut() {
            let newest = newest_mtime(p);
            if newest > cutoff {
                *kept = Some(format!("modified {}, within {days} days", ago(newest)));
            }
        }
    }

    if let Some(n) = c.keep_newest {
        let mut by_parent: BTreeMap<PathBuf, Vec<(usize, u64)>> = BTreeMap::new();
        for (i, (p, kept)) in out.iter().enumerate() {
            if kept.is_none() {
                let parent = p.parent().map(Path::to_path_buf).unwrap_or_default();
                by_parent.entry(parent).or_default().push((i, own_mtime(p)));
            }
        }
        for group in by_parent.values_mut() {
            group.sort_by_key(|&(_, mtime)| std::cmp::Reverse(mtime));
            for &(i, _) in group.iter().take(n) {
                out[i].1 = Some(if n == 1 {
                    "newest in its directory".to_string()
                } else {
                    format!("among the {n} newest in its directory")
                });
            }
        }
    }
    out
}

fn remove(path: &PathBuf) -> std::io::Result<()> {
    let md = std::fs::symlink_metadata(path)?;
    if md.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
}

/// Run one cleaner. Nothing is deleted unless `apply` is true.
pub fn run(c: &Cleaner, apply: bool) -> Outcome {
    let mut out = Outcome {
        name: c.name.clone(),
        status: Status::Nothing,
        reason: None,
        command: None,
        targets: Vec::new(),
        bytes: 0,
    };
    if let Some(proc_name) = &c.skip_if_running {
        if is_running(proc_name) {
            out.status = Status::Skipped;
            out.reason = Some(format!("{proc_name} is running"));
            return out;
        }
    }
    if let Some(spec) = c.orphans {
        match crate::orphans::find(spec) {
            Err(e) => {
                out.status = Status::Skipped;
                out.reason = Some(e);
                return out;
            }
            Ok(found) => {
                if found.is_empty() {
                    return out;
                }
                for path in found {
                    let bytes = disk_usage(&path);
                    let error = if apply {
                        remove(&path).err().map(|e| e.to_string())
                    } else {
                        None
                    };
                    if error.is_none() {
                        out.bytes += bytes;
                    }
                    out.targets.push(Target {
                        path,
                        bytes,
                        kept: None,
                        error,
                    });
                }
                out.status = if apply {
                    Status::Cleared
                } else {
                    Status::WouldClear
                };
                return out;
            }
        }
    }
    if !c.command.is_empty() {
        let argv = &c.command;
        out.command = Some(argv.join(" "));
        if !on_path(&argv[0]) {
            out.status = Status::Skipped;
            out.reason = Some(format!("{} is not installed", argv[0]));
            return out;
        }
        if !apply {
            out.status = Status::WouldRun;
            return out;
        }
        let status = Command::new(&argv[0])
            .args(&argv[1..])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        out.status = match status {
            Ok(s) if s.success() => Status::Ran,
            _ => Status::Failed,
        };
        return out;
    }

    let found = candidates(c);
    if found.is_empty() {
        return out;
    }
    for (path, kept) in found {
        let bytes = disk_usage(&path);
        let error = if apply && kept.is_none() {
            remove(&path).err().map(|e| e.to_string())
        } else {
            None
        };
        if error.is_none() && kept.is_none() {
            out.bytes += bytes;
        }
        out.targets.push(Target {
            path,
            bytes,
            kept,
            error,
        });
    }
    out.status = if apply {
        Status::Cleared
    } else {
        Status::WouldClear
    };
    out
}

pub fn print_text(c: &Cleaner, o: &Outcome) {
    println!("\n{}  ({})", c.name, c.summary);
    match o.status {
        Status::Skipped => println!("  skipped: {}", o.reason.as_deref().unwrap_or("")),
        Status::Nothing => println!("  nothing to clear"),
        Status::WouldRun => println!("  would run: {}", o.command.as_deref().unwrap_or("")),
        Status::Ran => println!("  ran: {}", o.command.as_deref().unwrap_or("")),
        Status::Failed => println!("  failed: {}", o.command.as_deref().unwrap_or("")),
        Status::WouldClear | Status::Cleared if o.targets.len() > 40 => {
            let verb = if o.status == Status::Cleared {
                "removed"
            } else {
                "would remove"
            };
            println!("  {verb} {} files, {}", o.targets.len(), human(o.bytes));
            println!("  every one of them checked against the application's own records");
        }
        Status::WouldClear | Status::Cleared => {
            for t in &o.targets {
                match &t.kept {
                    Some(why) => {
                        println!("  {:>9}  {}  kept: {why}", human(t.bytes), tilde(&t.path))
                    }
                    None => println!("  {:>9}  {}", human(t.bytes), tilde(&t.path)),
                }
                if let Some(e) = &t.error {
                    println!("             failed: {e}");
                }
            }
            let verb = if o.status == Status::Cleared {
                "reclaimed"
            } else {
                "would reclaim"
            };
            println!("  {verb} {}", human(o.bytes));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str, paths: &[&str]) -> Cleaner {
        Cleaner {
            source: Source::Config,
            ..super::paths(name, "", "because", None, paths)
        }
    }

    #[test]
    fn builtins_validate() {
        for c in builtins() {
            c.validate().unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn rejects_broad_or_odd_paths() {
        assert!(user("x", &["~/Library"]).validate().is_err());
        assert!(user("x", &["~/*"]).validate().is_err());
        assert!(user("x", &["~/*/Caches"]).validate().is_err());
        assert!(user("x", &["/tmp/foo/*"]).validate().is_err());
        assert!(user("x", &["~/a/../b"]).validate().is_err());
        assert!(user("x", &["~/Downloads/*.dmg"]).validate().is_ok());
        assert!(user("Bad Name", &["~/a/b"]).validate().is_err());
        let mut both = user("x", &["~/a/b"]);
        both.command = vec!["true".into()];
        assert!(both.validate().is_err());
    }

    #[test]
    fn keep_newest_and_older_than() {
        let root = std::env::temp_dir().join(format!("mac-headroom-cl-{}", now()));
        let base = root.join("arch");
        std::fs::create_dir_all(base.join("old")).unwrap();
        std::fs::create_dir_all(base.join("new")).unwrap();
        // Make "old" clearly older via a filetime set through `touch -t`.
        std::process::Command::new("touch")
            .args(["-t", "202001010000", base.join("old").to_str().unwrap()])
            .status()
            .unwrap();
        std::fs::write(base.join("new/f"), b"x").unwrap();

        // The candidate paths go through expand(), which only rewrites `~/`.
        let pattern = format!("{}/*/*", root.display());
        let mut c = super::paths("t", "", "because", None, &[&pattern]);
        c.keep_newest = Some(1);
        let found = candidates(&c);
        let kept: Vec<_> = found
            .iter()
            .filter(|(_, k)| k.is_some())
            .map(|(p, _)| p.clone())
            .collect();
        assert_eq!(kept, vec![base.join("new")]);

        c.keep_newest = None;
        c.older_than_days = Some(7);
        let found = candidates(&c);
        let deletable: Vec<_> = found
            .iter()
            .filter(|(_, k)| k.is_none())
            .map(|(p, _)| p.clone())
            .collect();
        assert_eq!(deletable, vec![base.join("old")]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
