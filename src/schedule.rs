//! Weekly launchd job. `install` writes a LaunchAgent that invokes
//! `mac-headroom schedule run` with the approved cleaners; `run` is what
//! launchd calls, and does the whole weekly routine: diagnose, growth scan,
//! clean. Everything it prints lands in ~/Library/Logs/mac-headroom.log.

use crate::util::{home, human, stdout_of};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub const LABEL: &str = "com.mac-headroom.schedule";
/// The hourly low-space check. Separate from the weekly job because launchd
/// runs one program per job, and because losing one should not lose the other.
pub const WATCH_LABEL: &str = "com.mac-headroom.watch";

pub fn plist_path() -> PathBuf {
    home()
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

pub fn watch_plist_path() -> PathBuf {
    home()
        .join("Library/LaunchAgents")
        .join(format!("{WATCH_LABEL}.plist"))
}

fn watch_plist(exe: &str, every_minutes: u64) -> String {
    let log = log_path().display().to_string();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{WATCH_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>check</string>
  </array>
  <key>StartInterval</key>
  <integer>{}</integer>
  <key>StandardErrorPath</key>
  <string>{log}</string>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#,
        xml_escape(exe),
        every_minutes * 60
    )
}

pub fn log_path() -> PathBuf {
    home().join("Library/Logs/mac-headroom.log")
}

/// One line per completed routine. The log only captures what launchd
/// redirects, so a run started by hand would otherwise leave no trace.
fn runs_path() -> PathBuf {
    crate::util::state_dir().join("runs.tsv")
}

fn record_run(before: Option<u64>, after: Option<u64>) {
    use std::io::Write;
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(runs_path())
    {
        let _ = writeln!(
            f,
            "{}\t{}\t{}",
            crate::util::now(),
            before.unwrap_or(0),
            after.unwrap_or(0)
        );
    }
}

fn last_run() -> Option<String> {
    let text = fs::read_to_string(runs_path()).ok()?;
    let line = text.lines().rev().find(|l| !l.trim().is_empty())?;
    let mut f = line.split('\t');
    let at: u64 = f.next()?.parse().ok()?;
    let before: u64 = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let after: u64 = f.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let stamp = stdout_of("date", &["-r", &at.to_string(), "+%-d %b %Y, %H:%M"]);
    let stamp = stamp.trim();
    Some(format!(
        "{} ({}) — {} free before, {} free after",
        if stamp.is_empty() {
            at.to_string()
        } else {
            stamp.to_string()
        },
        crate::util::ago(at),
        human(before),
        human(after)
    ))
}

fn uid() -> String {
    stdout_of("id", &["-u"]).trim().to_string()
}

fn domain_target() -> String {
    format!("gui/{}/{LABEL}", uid())
}

pub fn weekday_number(name: &str) -> Option<u8> {
    match name.to_ascii_lowercase().as_str() {
        "sun" | "sunday" => Some(0),
        "mon" | "monday" => Some(1),
        "tue" | "tuesday" => Some(2),
        "wed" | "wednesday" => Some(3),
        "thu" | "thursday" => Some(4),
        "fri" | "friday" => Some(5),
        "sat" | "saturday" => Some(6),
        _ => None,
    }
}

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn plist(exe: &str, weekday: u8, hour: u8, minute: u8, only: &[String], growth: bool) -> String {
    let mut args = vec![exe.to_string(), "schedule".into(), "run".into()];
    if !growth {
        args.push("--no-growth".into());
    }
    for name in only {
        args.push("--only".into());
        args.push(name.clone());
    }
    let args_xml: String = args
        .iter()
        .map(|a| format!("    <string>{}</string>\n", xml_escape(a)))
        .collect();
    let log = log_path().display().to_string();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{args_xml}  </array>
  <key>StartCalendarInterval</key>
  <dict>
    <key>Weekday</key>
    <integer>{weekday}</integer>
    <key>Hour</key>
    <integer>{hour}</integer>
    <key>Minute</key>
    <integer>{minute}</integer>
  </dict>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
  <key>RunAtLoad</key>
  <false/>
</dict>
</plist>
"#
    )
}

/// The path to record in a launch agent, preferring a name that will still
/// exist after an upgrade.
///
/// Resolving symlinks is the obvious thing to do and the wrong one. Homebrew
/// puts a symlink in its bin directory pointing into a Cellar directory named
/// for the version, so following it pins the job to a release that the next
/// `brew upgrade` deletes, and the job then fails silently every week. If a
/// directory on PATH holds a name that resolves to this same binary, that name
/// is the stable one and is what gets recorded.
fn stable_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let real = exe.canonicalize().unwrap_or_else(|_| exe.clone());
    let Some(name) = real.file_name() else {
        return Ok(real);
    };
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(name);
            if candidate == real {
                continue;
            }
            if candidate.is_file() && candidate.canonicalize().is_ok_and(|c| c == real) {
                return Ok(candidate);
            }
        }
    }
    Ok(real)
}

fn launchctl(args: &[&str]) -> bool {
    Command::new("launchctl")
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn is_loaded() -> bool {
    launchctl(&["print", &domain_target()])
}

fn watch_target() -> String {
    format!("gui/{}/{WATCH_LABEL}", uid())
}

fn watch_loaded() -> bool {
    launchctl(&["print", &watch_target()])
}

fn install_watch(exe: &str, every_minutes: u64) -> Result<(), String> {
    let path = watch_plist_path();
    if watch_loaded() {
        launchctl(&["bootout", &watch_target()]);
    }
    fs::write(&path, watch_plist(exe, every_minutes)).map_err(|e| e.to_string())?;
    let domain = format!("gui/{}", uid());
    if !launchctl(&["bootstrap", &domain, &path.to_string_lossy()]) {
        return Err(format!("launchctl bootstrap failed for {}", path.display()));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn install(
    weekday: &str,
    hour: u8,
    minute: u8,
    only: &[String],
    growth: bool,
    watch_minutes: Option<u64>,
) -> Result<(), String> {
    let wd = weekday_number(weekday).ok_or_else(|| format!("unknown weekday: {weekday}"))?;
    if hour > 23 || minute > 59 {
        return Err("hour must be 0-23 and minute 0-59".into());
    }
    let all = crate::config::all_cleaners()?;
    for name in only {
        if crate::cleaners::find(&all, name).is_none() {
            return Err(format!(
                "unknown cleaner: {name}  (see `mac-headroom list`)"
            ));
        }
    }
    let exe = stable_exe()?;
    let path = plist_path();
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Re-installing replaces the existing job.
    if is_loaded() {
        launchctl(&["bootout", &domain_target()]);
    }
    fs::write(
        &path,
        plist(&exe.to_string_lossy(), wd, hour, minute, only, growth),
    )
    .map_err(|e| e.to_string())?;
    let domain = format!("gui/{}", uid());
    if !launchctl(&["bootstrap", &domain, &path.to_string_lossy()]) {
        return Err(format!("launchctl bootstrap failed for {}", path.display()));
    }
    println!("Installed {LABEL}");
    println!(
        "  runs    every {} at {hour:02}:{minute:02} (or on next wake if asleep)",
        WEEKDAYS[wd as usize]
    );
    println!("  binary  {}", exe.display());
    println!("  plist   {}", path.display());
    println!("  log     {}", log_path().display());
    match watch_minutes {
        Some(m) => {
            install_watch(&exe.to_string_lossy(), m)?;
            println!(
                "  watches every {m} min, notifying below {:.0}% free",
                crate::config::alert_below_percent()
            );
        }
        None => {
            if watch_loaded() {
                launchctl(&["bootout", &watch_target()]);
            }
            let _ = fs::remove_file(watch_plist_path());
            println!("  no low space watch (--no-watch)");
        }
    }
    let what = if only.is_empty() {
        "all cleaners".to_string()
    } else {
        only.join(", ")
    };
    println!(
        "  cleans  {what}{}",
        if growth {
            ", plus a home growth scan"
        } else {
            ""
        }
    );
    if exe.components().any(|c| c.as_os_str() == "target") {
        println!(
            "WARNING: that binary is inside a cargo build directory and will vanish on `cargo clean`. Prefer `cargo install --path .` then reinstall."
        );
    }
    if exe.components().any(|c| c.as_os_str() == "Cellar") {
        println!(
            "WARNING: that path names a specific version and an upgrade will remove it. Re-run `mac-headroom schedule install` after upgrading."
        );
    }
    println!("Run it now with: mac-headroom schedule run");
    Ok(())
}

pub fn uninstall() -> Result<(), String> {
    let path = plist_path();
    let was_loaded = is_loaded();
    if was_loaded {
        launchctl(&["bootout", &domain_target()]);
    }
    let existed = path.exists();
    if existed {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    let watch_existed = watch_plist_path().exists() || watch_loaded();
    if watch_loaded() {
        launchctl(&["bootout", &watch_target()]);
    }
    let _ = fs::remove_file(watch_plist_path());
    if watch_existed {
        println!("Removed {WATCH_LABEL}");
    }
    if !was_loaded && !existed && !watch_existed {
        println!("Nothing installed.");
    } else {
        println!(
            "Removed {LABEL}. The log at {} is kept.",
            log_path().display()
        );
    }
    Ok(())
}

#[derive(Serialize)]
pub struct Status {
    pub installed: bool,
    pub loaded: bool,
    pub plist: PathBuf,
    pub log: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
    pub binary_exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    pub cleaners: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<String>,
    pub watch_installed: bool,
    pub watch_loaded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watch_every_minutes: Option<u64>,
    pub alert_below_percent: f64,
}

fn plist_strings(xml: &str, after_key: &str) -> Vec<String> {
    // Minimal read of our own plist: the <string> children of the array following a key.
    let Some(start) = xml.find(&format!("<key>{after_key}</key>")) else {
        return vec![];
    };
    let Some(end) = xml[start..].find("</array>") else {
        return vec![];
    };
    xml[start..start + end]
        .split("<string>")
        .skip(1)
        .filter_map(|s| s.split("</string>").next().map(xml_unescape))
        .collect()
}

fn plist_int_u64(xml: &str, key: &str) -> Option<u64> {
    let start = xml.find(&format!("<key>{key}</key>"))?;
    let rest = &xml[start..];
    let s = rest.find("<integer>")? + "<integer>".len();
    let e = rest[s..].find("</integer>")? + s;
    rest[s..e].parse().ok()
}

fn plist_int(xml: &str, key: &str) -> Option<u8> {
    let start = xml.find(&format!("<key>{key}</key>"))?;
    let rest = &xml[start..];
    let s = rest.find("<integer>")? + "<integer>".len();
    let e = rest[s..].find("</integer>")? + s;
    rest[s..e].parse().ok()
}

pub fn status() -> Status {
    let plist = plist_path();
    let xml = fs::read_to_string(&plist).unwrap_or_default();
    let args = plist_strings(&xml, "ProgramArguments");
    let binary = args.first().cloned();
    let cleaners = args
        .windows(2)
        .filter(|w| w[0] == "--only")
        .map(|w| w[1].clone())
        .collect();
    let schedule = match (
        plist_int(&xml, "Weekday"),
        plist_int(&xml, "Hour"),
        plist_int(&xml, "Minute"),
    ) {
        (Some(w), Some(h), Some(m)) => {
            Some(format!("{} {h:02}:{m:02}", WEEKDAYS[(w % 7) as usize]))
        }
        _ => None,
    };
    let last_run = last_run().or_else(|| {
        fs::read_to_string(log_path()).ok().and_then(|s| {
            s.lines()
                .rev()
                .find(|l| l.contains("=== done"))
                .map(str::to_string)
        })
    });
    Status {
        installed: plist.exists(),
        loaded: is_loaded(),
        binary_exists: binary.as_ref().is_some_and(|b| PathBuf::from(b).exists()),
        binary,
        plist,
        log: log_path(),
        schedule,
        cleaners,
        last_run,
        watch_installed: watch_plist_path().exists(),
        watch_loaded: watch_loaded(),
        watch_every_minutes: fs::read_to_string(watch_plist_path())
            .ok()
            .and_then(|x| plist_int_u64(&x, "StartInterval"))
            .map(|s| s / 60),
        alert_below_percent: crate::config::alert_below_percent(),
    }
}

pub fn print_status(s: &Status) {
    if !s.installed {
        println!("Not installed. Install with: mac-headroom schedule install");
        return;
    }
    println!("{LABEL}");
    println!(
        "  loaded   {}",
        if s.loaded {
            "yes"
        } else {
            "NO (plist exists but launchd does not have it)"
        }
    );
    println!("  runs     {}", s.schedule.as_deref().unwrap_or("?"));
    let what = if s.cleaners.is_empty() {
        "all cleaners".to_string()
    } else {
        s.cleaners.join(", ")
    };
    println!("  cleans   {what}");
    let bin = s.binary.as_deref().unwrap_or("?");
    if bin.contains("/Cellar/") {
        println!(
            "  NOTE     that path names one version; re-run schedule install after an upgrade"
        );
    }
    println!(
        "  binary   {bin}{}",
        if s.binary_exists {
            ""
        } else {
            "  (MISSING: reinstall)"
        }
    );
    println!("  log      {}", s.log.display());
    match &s.last_run {
        Some(l) => println!("  last run {l}"),
        None => println!("  last run never"),
    }
    match (s.watch_installed, s.watch_every_minutes) {
        (true, Some(m)) => println!(
            "  watch    every {m} min, notifies below {:.0}% free{}",
            s.alert_below_percent,
            if s.watch_loaded { "" } else { "  (NOT loaded)" }
        ),
        _ => println!("  watch    not installed"),
    }
}

/// launchd does not load the shell profile, so command cleaners (npm, brew, go...)
/// would be silently skipped as "not installed". Add the usual places.
fn extend_path() {
    let h = home();
    let mut extra: Vec<PathBuf> = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        h.join(".local/bin"),
        h.join(".cargo/bin"),
        h.join("go/bin"),
    ];
    // Newest nvm node, if any.
    if let Ok(rd) = fs::read_dir(h.join(".nvm/versions/node")) {
        if let Some(newest) = rd.flatten().map(|e| e.path()).max() {
            extra.push(newest.join("bin"));
        }
    }
    let current = std::env::var_os("PATH").unwrap_or_default();
    let mut all = extra;
    all.extend(std::env::split_paths(&current));
    if let Ok(joined) = std::env::join_paths(all) {
        std::env::set_var("PATH", joined);
    }
}

fn stamp() -> String {
    stdout_of("date", &["+%Y-%m-%d %H:%M:%S"])
        .trim()
        .to_string()
}

/// The weekly routine. Prints text; launchd sends it to the log.
pub fn run(only: &[String], growth: bool) {
    extend_path();
    println!("[{}] === mac-headroom scheduled run ===", stamp());
    let free_before = crate::diag::disk().map(|d| d.free);

    match crate::diag::report() {
        Some(r) => crate::diag::print_text(&r),
        None => println!("diagnose failed: could not read diskutil"),
    }

    if growth {
        for (i, root) in std::iter::once(home())
            .chain(crate::config::scan_roots())
            .enumerate()
        {
            println!();
            if i > 0 {
                println!("(extra scan root from the config)");
            }
            crate::growth::print_text(&crate::growth::report(&root, 3, 100 << 20, 15));
        }
    }

    println!();
    let report = crate::run_clean(false, true, only);
    let free_after = crate::diag::disk().map(|d| d.free);
    record_run(free_before, free_after);
    // Leave a current report behind, so the weekly picture is ready to open.
    let report_path = crate::report::default_path();
    match crate::report::write(&crate::report::gather(), &report_path) {
        Ok(()) => println!("\nreport: {}", report_path.display()),
        Err(e) => println!("\ncould not write report: {e}"),
    }
    println!();
    match (free_before, free_after) {
        (Some(b), Some(a)) => println!(
            "[{}] === done: {} free -> {} free ===",
            stamp(),
            human(b),
            human(a)
        ),
        _ => println!(
            "[{}] === done: {} reclaimed ===",
            stamp(),
            human(report.bytes)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_a_stable_name_over_a_versioned_one() {
        // A symlink on PATH standing in for Homebrew's bin entry.
        let root = std::env::temp_dir().join(format!("mh-exe-{:?}", std::thread::current().id()));
        let _ = fs::remove_dir_all(&root);
        let cellar = root.join("Cellar/thing/1.2.3/bin");
        let bin = root.join("bin");
        fs::create_dir_all(&cellar).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let real = cellar.join("thing");
        fs::write(&real, b"#!/bin/sh\n").unwrap();
        std::os::unix::fs::symlink(&real, bin.join("thing")).unwrap();

        // stable_exe looks the current binary up by name on PATH; emulate that
        // resolution directly, since the test binary is not the one installed.
        let found = std::env::split_paths(&bin.clone().into_os_string())
            .map(|d| d.join("thing"))
            .find(|c| {
                c.is_file()
                    && c.canonicalize()
                        .is_ok_and(|r| r == real.canonicalize().unwrap())
            })
            .unwrap();
        assert_eq!(
            found,
            bin.join("thing"),
            "the stable name, not the Cellar path"
        );
        assert!(!found.components().any(|c| c.as_os_str() == "Cellar"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn weekday_names() {
        assert_eq!(weekday_number("Mon"), Some(1));
        assert_eq!(weekday_number("sunday"), Some(0));
        assert_eq!(weekday_number("someday"), None);
    }

    #[test]
    fn plist_round_trips() {
        let xml = plist(
            "/usr/local/bin/mac-headroom",
            1,
            10,
            30,
            &["a".into(), "b<c".into()],
            true,
        );
        assert_eq!(
            plist_strings(&xml, "ProgramArguments"),
            vec![
                "/usr/local/bin/mac-headroom",
                "schedule",
                "run",
                "--only",
                "a",
                "--only",
                "b<c"
            ]
        );
        assert_eq!(plist_int(&xml, "Weekday"), Some(1));
        assert_eq!(plist_int(&xml, "Hour"), Some(10));
        assert_eq!(plist_int(&xml, "Minute"), Some(30));
        assert!(xml.contains("&lt;"));
    }
}
