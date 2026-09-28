//! Apps nobody has opened for months.
//!
//! An IDE tried once, a game finished with, a tool that came with a course:
//! each sits in /Applications at a gigabyte or two, and nothing on the Mac
//! ever says so. This lists them, with when each was last opened, and the
//! command that moves one to the Trash. Never a deletion: the Trash keeps an
//! app until it is emptied, so a wrong call costs a drag back.
//!
//! "Last opened" is the latest of several signals, because no single one is
//! there for every app. Spotlight's `kMDItemLastUsedDate` is missing for a
//! third of the apps on the author's Mac (Firefox, Zoom, Anki among them), so
//! the modification times of the app's own preferences, Application Support,
//! caches and saved state are read too. An app with none of these is reported
//! as unknown, never as unused: no record is not the same as no use.
//! An app with anything of its own running, a VPN's daemon say, is in use and
//! is not listed at all.

use crate::findings::Item;
use crate::util::{disk_usage, home, now};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::UNIX_EPOCH;

/// How long without a launch before an app is listed.
pub const UNUSED_AFTER: u64 = 180 * 86_400;

/// Apps smaller than this are not worth someone's attention one by one.
const MIN_APP: u64 = 50_000_000;

/// An app with no record of a launch is a weaker case, so it is listed only
/// when it is large enough that a wrong guess costs real space. Under this,
/// a row saying "nothing records when it was last opened" was noise.
const MIN_UNKNOWN_APP: u64 = 500_000_000;

/// What can be said about an app that has not been opened lately.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Last opened this long ago, going by every signal there is.
    Unused { last_used: u64 },
    /// Nothing on the Mac records a launch. The tool does not guess.
    Unknown,
}

/// An app worth listing: where it is, how big, and the verdict.
pub struct UnusedApp {
    pub path: PathBuf,
    pub bytes: u64,
    pub verdict: Verdict,
}

/// The app bundles at the top of /Applications and ~/Applications. Folders
/// (Utilities, a vendor's own) are not walked: an app inside one is that
/// vendor's to arrange. Symlinks are skipped, which is how Safari and the
/// other apps macOS keeps on its sealed volume appear there.
fn installed() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in [PathBuf::from("/Applications"), home().join("Applications")] {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let is_app = path.extension().is_some_and(|e| e == "app");
            let real_dir = fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir());
            if is_app && real_dir {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Spotlight's answers for one app: its bundle identifier and when it was
/// last opened, either of which may be missing.
fn spotlight(app: &Path) -> (Option<String>, Option<u64>) {
    let out = Command::new("mdls")
        .args([
            "-name",
            "kMDItemCFBundleIdentifier",
            "-name",
            "kMDItemLastUsedDate",
            "-raw",
        ])
        .arg(app)
        .output();
    let Ok(out) = out else {
        return (None, None);
    };
    let text = String::from_utf8_lossy(&out.stdout);
    // `-raw` prints the values in the order asked, separated by NUL.
    let mut fields = text.split('\0');
    let present = |s: Option<&str>| {
        s.map(str::trim)
            .filter(|s| !s.is_empty() && *s != "(null)")
            .map(str::to_string)
    };
    let bundle_id = present(fields.next());
    let last_used = present(fields.next()).and_then(|d| parse_mdls_date(&d));
    (bundle_id, last_used)
}

/// `2026-09-01 11:38:21 +0000`, as mdls prints it, to seconds since 1970.
/// Spotlight always prints UTC, and a day either way would not change the
/// verdict, so only that form is read.
fn parse_mdls_date(text: &str) -> Option<u64> {
    let mut parts = text.split_whitespace();
    let date = parts.next()?;
    let time = parts.next()?;
    let mut ymd = date.split('-').map(|n| n.parse::<i64>().ok());
    let (y, m, d) = (ymd.next()??, ymd.next()??, ymd.next()??);
    let mut hms = time.split(':').map(|n| n.parse::<i64>().ok());
    let (h, mi, s) = (hms.next()??, hms.next()??, hms.next()??);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // Days from 1970-01-01 to the civil date (Howard Hinnant's algorithm).
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3600 + mi * 60 + s;
    u64::try_from(secs).ok()
}

fn mtime(path: &Path) -> Option<u64> {
    if crate::util::inside_protected(path) {
        return None;
    }
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// The files an app writes when it runs: preferences on launch and quit,
/// state when it closes, caches while it works. The newest of them, with
/// Spotlight's date, is the best answer this Mac has for "last opened".
fn last_used(app: &Path, bundle_id: Option<&str>, from_spotlight: Option<u64>) -> Option<u64> {
    let h = home();
    let name = app_name(app);
    let mut probes = vec![h.join("Library/Application Support").join(&name)];
    if let Some(id) = bundle_id {
        probes.extend([
            h.join("Library/Preferences").join(format!("{id}.plist")),
            h.join("Library/Application Support").join(id),
            h.join("Library/Caches").join(id),
            h.join("Library/HTTPStorages").join(id),
            h.join("Library/Saved Application State")
                .join(format!("{id}.savedState")),
            h.join("Library/Containers")
                .join(id)
                .join("Data/Library/Preferences")
                .join(format!("{id}.plist")),
        ]);
    }
    probes
        .iter()
        .filter_map(|p| mtime(p))
        .chain(from_spotlight)
        .max()
}

/// "PyCharm", from "/Applications/PyCharm.app".
pub fn app_name(app: &Path) -> String {
    app.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Whether anything inside each bundle is running, by app. One `ps` for
/// every app rather than a `pgrep` each. A helper or a daemon that an app
/// installed counts as the app in use: a VPN whose daemon runs is doing its
/// job whether or not its window was ever opened, so it is not listed.
fn running_inside(apps: &[&Path]) -> Vec<bool> {
    let listing = Command::new("ps")
        .args(["-axo", "comm="])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    apps.iter()
        .map(|app| {
            let prefix = format!("{}/", app.display());
            listing
                .lines()
                .map(str::trim)
                .any(|line| line.starts_with(&prefix))
        })
        .collect()
}

/// Every app worth listing, largest first. Only apps that look unused are
/// measured, because measuring means walking the bundle and Xcode's takes
/// seconds.
pub fn find() -> Vec<UnusedApp> {
    let cutoff = now().saturating_sub(UNUSED_AFTER);
    let mut stale: Vec<(PathBuf, Option<u64>)> = Vec::new();
    for app in installed() {
        let (bundle_id, from_spotlight) = spotlight(&app);
        let used = last_used(&app, bundle_id.as_deref(), from_spotlight);
        if used.is_none_or(|at| at < cutoff) {
            stale.push((app, used));
        }
    }
    let paths: Vec<&Path> = stale.iter().map(|(p, _)| p.as_path()).collect();
    let running = running_inside(&paths);
    let mut out: Vec<UnusedApp> = stale
        .iter()
        .zip(running)
        .filter(|(_, running)| !running)
        .filter_map(|((path, used), _)| {
            let bytes = disk_usage(path);
            let verdict = match used {
                Some(last_used) => Verdict::Unused {
                    last_used: *last_used,
                },
                None => Verdict::Unknown,
            };
            worth_listing(&verdict, bytes).then(|| UnusedApp {
                path: path.clone(),
                bytes,
                verdict,
            })
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.bytes));
    out
}

/// A known case from 50 MB; an unknown one only from 500 MB.
fn worth_listing(verdict: &Verdict, bytes: u64) -> bool {
    match verdict {
        Verdict::Unused { .. } => bytes >= MIN_APP,
        Verdict::Unknown => bytes >= MIN_UNKNOWN_APP,
    }
}

/// "about 8 months ago", "over a year ago", "about 2 years ago". Coarse on
/// purpose: the day an app was last opened is not the point, the months are.
pub fn months_ago(epoch: u64, now: u64) -> String {
    let months = now.saturating_sub(epoch) / (30 * 86_400);
    match months {
        0..=11 => format!("about {} months ago", months.max(6)),
        12..=17 => "over a year ago".to_string(),
        18..=23 => "about a year and a half ago".to_string(),
        _ => format!("about {} years ago", months / 12),
    }
}

/// Moves the app to the Trash the way Finder does, so Put Back works, and
/// asks nothing of the person but a paste. The path goes inside an
/// AppleScript string, then the whole script inside a shell single-quoted
/// argument, so both kinds of quoting are done here.
pub fn trash_command(app: &Path) -> String {
    let inner = app
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let script = format!("tell application \"Finder\" to delete POSIX file \"{inner}\"");
    format!("osascript -e '{}'", script.replace('\'', "'\\''"))
}

/// The rows under the detection, one per app.
pub fn items(apps: &[UnusedApp], now: u64) -> Vec<Item> {
    apps.iter()
        .map(|app| {
            let (safe, note) = match &app.verdict {
                Verdict::Unused { last_used } => (
                    true,
                    format!(
                        "Last opened {}. The Trash keeps it until you empty it, so you can put it back.",
                        months_ago(*last_used, now)
                    ),
                ),
                Verdict::Unknown => (
                    false,
                    "Nothing on this Mac records when it was last opened, so decide for yourself before removing it."
                        .to_string(),
                ),
            };
            Item {
                label: Some(app_name(&app.path)),
                path: app.path.display().to_string(),
                bytes: app.bytes,
                safe,
                note,
                command: if safe {
                    trash_command(&app.path)
                } else {
                    String::new()
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_date_mdls_prints() {
        assert_eq!(
            parse_mdls_date("2026-09-01 11:38:21 +0000"),
            Some(1_788_262_701)
        );
        assert_eq!(parse_mdls_date("1970-01-01 00:00:00 +0000"), Some(0));
        assert_eq!(
            parse_mdls_date("2000-02-29 00:00:00 +0000"),
            Some(951_782_400)
        );
        assert_eq!(parse_mdls_date("(null)"), None);
        assert_eq!(parse_mdls_date("2026-13-01 00:00:00 +0000"), None);
    }

    #[test]
    fn months_are_coarse_and_never_under_the_cutoff() {
        let now = 2_000_000_000;
        let month = 30 * 86_400;
        // Listed only after six months, so a shorter reading is a rounding
        // quirk and reads as six.
        assert_eq!(
            months_ago(now - 6 * month + 86_400, now),
            "about 6 months ago"
        );
        assert_eq!(months_ago(now - 8 * month, now), "about 8 months ago");
        assert_eq!(months_ago(now - 14 * month, now), "over a year ago");
        assert_eq!(
            months_ago(now - 20 * month, now),
            "about a year and a half ago"
        );
        assert_eq!(months_ago(now - 30 * month, now), "about 2 years ago");
    }

    #[test]
    fn the_trash_command_survives_quotes_in_the_name() {
        assert_eq!(
            trash_command(Path::new("/Applications/PyCharm CE.app")),
            r#"osascript -e 'tell application "Finder" to delete POSIX file "/Applications/PyCharm CE.app"'"#
        );
        let odd = trash_command(Path::new("/Applications/Bob's \"Fun\" App.app"));
        assert!(odd.contains(r#"Bob'\''s \"Fun\" App.app"#), "{odd}");
    }

    #[test]
    fn an_app_with_no_record_needs_to_be_large_to_be_worth_a_row() {
        let unused = Verdict::Unused { last_used: 5 };
        assert!(worth_listing(&unused, 60_000_000));
        assert!(!worth_listing(&unused, 40_000_000));
        assert!(
            !worth_listing(&Verdict::Unknown, 240_000_000),
            "a 240 MB mystery is noise"
        );
        assert!(worth_listing(&Verdict::Unknown, 600_000_000));
    }

    #[test]
    fn only_a_verdict_of_unused_gets_a_command() {
        let apps = [
            UnusedApp {
                path: "/Applications/Old.app".into(),
                bytes: 1,
                verdict: Verdict::Unused { last_used: 0 },
            },
            UnusedApp {
                path: "/Applications/Mystery.app".into(),
                bytes: 1,
                verdict: Verdict::Unknown,
            },
        ];
        let items = items(&apps, 400 * 86_400);
        assert!(items[0].safe && items[0].command.starts_with("osascript"));
        assert_eq!(items[0].label.as_deref(), Some("Old"));
        assert!(items[0].note.starts_with("Last opened over a year ago"));
        assert!(!items[1].safe && items[1].command.is_empty());
    }
}
