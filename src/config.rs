//! User configuration: extra cleaners and disabled built-ins, in TOML.

use crate::cleaners::{builtins, Cleaner, Source};
use crate::util::home;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

fn default_alert() -> f64 {
    5.0
}

pub fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"));
    base.join("mac-headroom/config.toml")
}

#[derive(Deserialize, Serialize, Default, Debug)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Built-in cleaners to leave out of a plain `clean`. Still runnable with --only.
    #[serde(default)]
    pub disable: Vec<String>,
    /// Extra directories to include in the space breakdown, beyond your home
    /// folder. Absolute paths. Scanning a system directory without sudo skips
    /// what it cannot read, and the report says so.
    /// Replaces the default list when set. Leave it out to use the defaults;
    /// set it to an empty list to scan only the home folder.
    #[serde(default)]
    pub scan_roots: Option<Vec<String>>,
    /// Notify when free space falls below this share of the disk. 0 turns it off.
    #[serde(default = "default_alert")]
    pub alert_below_percent: f64,
    #[serde(default, rename = "cleaner")]
    pub cleaners: Vec<Cleaner>,
}

/// Missing file is an empty config. A file that exists but does not parse is an error.
///
/// An absent file is parsed as an empty document rather than built with
/// `Config::default()`, because a derived Default ignores every
/// `#[serde(default = ...)]` and would quietly zero the alert threshold.
pub fn load() -> Result<Config, String> {
    let p = path();
    if !p.exists() {
        return toml::from_str("").map_err(|e| e.to_string());
    }
    let text = fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    toml::from_str(&text).map_err(|e| format!("{}:\n{e}", p.display()))
}

/// What a config says about itself. `problems` stop the tool; `notes` are
/// worth mentioning but are not errors.
#[derive(Default)]
pub struct Findings {
    pub problems: Vec<String>,
    pub notes: Vec<String>,
}

pub fn validate(cfg: &Config) -> Findings {
    let builtin = builtins();
    let mut problems = Vec::new();
    let mut notes = Vec::new();
    for d in &cfg.disable {
        if !builtin.iter().any(|b| &b.name == d) {
            problems.push(format!(
                "disable: {d:?} is not a built-in cleaner (see `mac-headroom list`)"
            ));
        }
    }
    if !(0.0..=90.0).contains(&cfg.alert_below_percent) {
        problems.push("alert_below_percent must be between 0 and 90".into());
    }
    for r in cfg.scan_roots.iter().flatten() {
        let p = std::path::Path::new(r);
        if !p.is_absolute() {
            problems.push(format!("scan_roots: {r:?} must be an absolute path"));
        } else if !p.is_dir() {
            notes.push(format!(
                "scan_roots: {r:?} is not on this machine, so it is skipped"
            ));
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for c in &cfg.cleaners {
        if let Err(e) = c.validate() {
            problems.push(e);
        }
        if builtin.iter().any(|b| b.name == c.name) {
            problems.push(format!("{}: name clashes with a built-in cleaner", c.name));
        }
        if !seen.insert(c.name.clone()) {
            problems.push(format!("{}: defined twice", c.name));
        }
    }
    Findings { problems, notes }
}

/// Built-ins (flagged if disabled) followed by the user's cleaners.
pub fn merge(cfg: &Config) -> Vec<Cleaner> {
    let mut all: Vec<Cleaner> = builtins()
        .into_iter()
        .map(|mut b| {
            b.disabled = cfg.disable.contains(&b.name);
            b
        })
        .collect();
    all.extend(cfg.cleaners.iter().cloned().map(|mut c| {
        c.source = Source::Config;
        c
    }));
    all
}

/// The cleaner set every command works from. Refuses to proceed on a bad config,
/// because a half-understood config is exactly how the wrong thing gets deleted.
/// The low-space threshold, as a share of the whole disk.
pub fn alert_below_percent() -> f64 {
    load()
        .map(|c| c.alert_below_percent)
        .unwrap_or_else(|_| default_alert())
}

/// Locations outside the home folder scanned when the config does not say
/// otherwise. Between them they cover nearly everything on the data volume that
/// a person without a config file would otherwise see as one unexplained lump.
pub const DEFAULT_SCAN_ROOTS: &[&str] = &[
    "/Applications",
    "/Library",
    "/System/Library/AssetsV2",
    "/opt",
    "/usr/local",
    "/private/var",
    "/private/tmp",
];

/// The roots actually scanned, beyond the home folder: the config's list if it
/// has one, otherwise the defaults that exist here and live on the data volume.
pub fn scan_roots() -> Vec<PathBuf> {
    match load().ok().and_then(|c| c.scan_roots) {
        Some(list) => list
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .collect(),
        None => DEFAULT_SCAN_ROOTS
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.is_dir() && crate::util::on_data_volume(p))
            .collect(),
    }
}

pub fn all_cleaners() -> Result<Vec<Cleaner>, String> {
    let cfg = load()?;
    let found = validate(&cfg);
    if !found.problems.is_empty() {
        return Err(format!(
            "{} has problems:\n  {}",
            path().display(),
            found.problems.join("\n  ")
        ));
    }
    Ok(merge(&cfg))
}

pub const EXAMPLE: &str = r#"# mac-headroom configuration
#
# Cleaners defined here run alongside the built-ins in `mac-headroom clean`,
# and can be named with --only (including in `schedule install`).
# Nothing is deleted without --yes, and every cleaner must say why it is safe.
# Check this file with: mac-headroom config check

# Built-in cleaners to leave out of a plain `clean`. They can still be run
# explicitly with --only. Names are in `mac-headroom list`.
disable = []

# Folders outside your home folder to include in the space breakdown. Leave this
# out to use the defaults: /Applications, /Library, /System/Library/AssetsV2,
# /opt, /usr/local, /private/var and /private/tmp, wherever they exist. Setting
# it replaces that list entirely; an empty list scans only your home folder.
# scan_roots = ["/Applications", "/Library"]

# Notify when free space drops below this share of the disk, checked hourly by
# the watch job that `schedule install` sets up. 0 turns notifications off.
alert_below_percent = 5

# ---- Examples. Remove the leading # to enable one. ----

# Plain globs: every match is deleted.
# Paths must start with ~/ and name a directory under ~ plus something inside it.
#
# [[cleaner]]
# name = "downloads-dmg"
# summary = "Installer images left in Downloads"
# why_safe = "A .dmg is only needed until the app is installed. Re-download if ever needed."
# paths = ["~/Downloads/*.dmg"]

# keep_newest = N: in each directory, keep the N most recently modified matches.
# For build caches that leak one copy per build, where only the newest is live.
#
# [[cleaner]]
# name = "myapp-model-cache"
# summary = "Stale compiled model bundles from MyApp dev builds"
# why_safe = "Only the newest bundle per arch is used. Older siblings are leftovers; the app recompiles if needed."
# paths = ["~/Library/Caches/com.example.myapp/models/*/*"]
# keep_newest = 1
# skip_if_running = "MyApp"

# older_than_days = N: only delete a match when nothing inside it was modified
# in the last N days. For scratch space that is safe once a job has finished.
#
# [[cleaner]]
# name = "agent-job-scratch"
# summary = "Scratch dirs left by finished background agent jobs"
# why_safe = "Untouched for a week means the job is long done and nothing reads it."
# paths = ["~/.claude/jobs/*/tmp"]
# older_than_days = 7

# command: run a tool's own cache clean instead of deleting paths.
#
# [[cleaner]]
# name = "yarn-cache"
# summary = "Yarn package cache"
# why_safe = "yarn's own cache clean. Packages re-download on the next install."
# command = ["yarn", "cache", "clean"]
"#;

pub fn init() -> Result<PathBuf, String> {
    let p = path();
    if p.exists() {
        return Err(format!("{} already exists", p.display()));
    }
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    fs::write(&p, EXAMPLE).map_err(|e| e.to_string())?;
    Ok(p)
}

#[derive(Serialize)]
pub struct Check {
    pub path: PathBuf,
    pub exists: bool,
    pub problems: Vec<String>,
    pub notes: Vec<String>,
    pub disabled: Vec<String>,
    pub user_cleaners: Vec<String>,
    pub scan_roots: Vec<String>,
    /// True when scan_roots come from the defaults rather than the config.
    pub default_scan_roots: bool,
    pub alert_below_percent: f64,
}

pub fn check() -> Check {
    let p = path();
    let exists = p.exists();
    match load() {
        Ok(cfg) => Check {
            path: p,
            exists,
            problems: validate(&cfg).problems,
            notes: validate(&cfg).notes,
            disabled: cfg.disable.clone(),
            user_cleaners: cfg.cleaners.iter().map(|c| c.name.clone()).collect(),
            scan_roots: scan_roots()
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            default_scan_roots: cfg.scan_roots.is_none(),
            alert_below_percent: cfg.alert_below_percent,
        },
        Err(e) => Check {
            path: p,
            exists,
            problems: vec![e],
            notes: vec![],
            disabled: vec![],
            user_cleaners: vec![],
            scan_roots: vec![],
            default_scan_roots: true,
            alert_below_percent: 0.0,
        },
    }
}

pub fn print_check(c: &Check) {
    println!(
        "Config  {}{}",
        c.path.display(),
        if c.exists {
            ""
        } else {
            "  (not present, using built-ins only)"
        }
    );
    if !c.disabled.is_empty() {
        println!("  disabled built-ins  {}", c.disabled.join(", "));
    }
    if !c.user_cleaners.is_empty() {
        println!("  user cleaners       {}", c.user_cleaners.join(", "));
    }
    if !c.scan_roots.is_empty() {
        println!(
            "  scan roots          {}{}",
            c.scan_roots.join(", "),
            if c.default_scan_roots {
                "  (defaults)"
            } else {
                ""
            }
        );
    }
    println!(
        "  low space alert     {}",
        if c.alert_below_percent > 0.0 {
            format!("under {:.0}% free", c.alert_below_percent)
        } else {
            "off".into()
        }
    );
    for n in &c.notes {
        println!("  note     {n}");
    }
    if c.problems.is_empty() {
        println!("  OK");
    } else {
        for p in &c.problems {
            println!("  PROBLEM  {p}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_parses_and_is_empty() {
        let cfg: Config = toml::from_str(EXAMPLE).unwrap();
        assert!(cfg.cleaners.is_empty());
        assert_eq!(
            cfg.scan_roots, None,
            "the example leaves the defaults in place"
        );
        assert_eq!(cfg.alert_below_percent, 5.0);
        assert!(validate(&cfg).problems.is_empty());

        // An absent config must still carry the defaults; a derived Default
        // would silently switch alerting off.
        let empty: Config = toml::from_str("").unwrap();
        assert_eq!(empty.alert_below_percent, 5.0);
        assert_eq!(empty.scan_roots, None);
    }

    #[test]
    fn missing_scan_root_is_a_note_not_a_problem() {
        let cfg: Config =
            toml::from_str(r#"scan_roots = ["/definitely/not/here", "relative/path"]"#).unwrap();
        let f = validate(&cfg);
        assert_eq!(f.notes.len(), 1, "{:?}", f.notes);
        assert_eq!(f.problems.len(), 1, "{:?}", f.problems);
    }

    #[test]
    fn example_entries_validate_when_enabled() {
        let enabled: String = EXAMPLE
            .lines()
            .map(|l| {
                l.strip_prefix("# ")
                    .or_else(|| l.strip_prefix("#"))
                    .unwrap_or(l)
            })
            .filter(|l| !l.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        // Only the [[cleaner]] tables and their fields survive; prose lines become
        // invalid TOML, so extract just the table blocks.
        let mut blocks = String::new();
        let mut inside = false;
        for l in enabled.lines() {
            if l.trim() == "[[cleaner]]" {
                inside = true;
            } else if l.trim().is_empty() {
                inside = false;
            }
            if inside {
                blocks.push_str(l);
                blocks.push('\n');
            }
        }
        let cfg: Config = toml::from_str(&blocks).unwrap_or_else(|e| panic!("{e}\n{blocks}"));
        assert_eq!(cfg.cleaners.len(), 4);
        assert!(
            validate(&cfg).problems.is_empty(),
            "{:?}",
            validate(&cfg).problems
        );
    }

    #[test]
    fn rejects_clash_and_duplicate() {
        let cfg: Config = toml::from_str(
            r#"
disable = ["nope"]
[[cleaner]]
name = "homebrew"
why_safe = "x"
paths = ["~/a/b"]
[[cleaner]]
name = "dup"
why_safe = "x"
paths = ["~/a/b"]
[[cleaner]]
name = "dup"
why_safe = "x"
paths = ["~/a/b"]
"#,
        )
        .unwrap();
        let problems = validate(&cfg).problems;
        assert_eq!(problems.len(), 3, "{problems:?}");
    }
}
