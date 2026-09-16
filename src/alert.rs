//! Noticing that the disk is filling up before it is too late.
//!
//! `check` is deliberately cheap: one `diskutil` call and no directory walk, so
//! it can run every hour. When free space falls below the configured share of
//! the disk it writes a fresh report and posts a notification, then holds its
//! tongue for a cooldown so a full disk does not nag every hour.

use crate::util::{human, now, state_dir};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// How long to stay quiet after alerting, so one full disk is one notification.
const COOLDOWN_SECS: u64 = 12 * 3600;

fn last_alert_path() -> PathBuf {
    state_dir().join("last-alert")
}

fn last_alert() -> Option<u64> {
    fs::read_to_string(last_alert_path())
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn record_alert(at: u64) {
    let _ = fs::write(last_alert_path(), at.to_string());
}

/// Post a macOS notification. Returns false if osascript is unavailable or
/// notifications are refused, which is not worth failing the run over.
pub fn notify(title: &str, body: &str) -> bool {
    // Quoting: these strings are ours, but a path could still carry a quote.
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        esc(body),
        esc(title)
    );
    Command::new("osascript")
        .args(["-e", &script])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[derive(Serialize)]
pub struct Check {
    pub free: u64,
    pub total: u64,
    pub free_percent: f64,
    pub threshold_percent: f64,
    pub low: bool,
    pub notified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_until: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<PathBuf>,
}

/// One cheap look at the disk. Writes a report and notifies only when free
/// space is below the threshold and the cooldown has passed, unless forced.
pub fn check(force: bool) -> Option<Check> {
    let d = crate::diag::disk()?;
    let threshold_percent = crate::config::alert_below_percent();
    let free_percent = 100.0 * d.free as f64 / d.total.max(1) as f64;
    let low = threshold_percent > 0.0 && free_percent < threshold_percent;

    let at = now();
    let quiet = last_alert().filter(|t| at.saturating_sub(*t) < COOLDOWN_SECS);
    let mut out = Check {
        free: d.free,
        total: d.total,
        free_percent,
        threshold_percent,
        low,
        notified: false,
        quiet_until: quiet.map(|t| t + COOLDOWN_SECS),
        report: None,
    };
    if !low || (quiet.is_some() && !force) {
        return Some(out);
    }

    // A notification the reader cannot act on is noise, so write the report first.
    let path = crate::report::default_path();
    if crate::report::write(&crate::report::gather(), &path).is_ok() {
        out.report = Some(path.clone());
    }
    let title = format!("{} free on this Mac", human(d.free));
    let body = format!(
        "Under {threshold_percent:.0}% of the disk is free. A report is ready at {}.",
        path.display()
    );
    out.notified = notify(&title, &body);
    record_alert(at);
    Some(out)
}

pub fn print_text(c: &Check) {
    println!(
        "{} free of {} ({:.1}%), alerting under {:.0}%",
        human(c.free),
        human(c.total),
        c.free_percent,
        c.threshold_percent
    );
    if c.threshold_percent <= 0.0 {
        println!("Alerts are switched off (alert_below_percent = 0).");
    } else if !c.low {
        println!("Above the threshold. Nothing to do.");
    } else if c.notified {
        println!("Low on space. Notified, and wrote a report.");
    } else if c.quiet_until.is_some() {
        println!("Low on space, but a notification already went out recently. Pass --force to send another.");
    } else {
        println!("Low on space. The notification could not be posted; a report was still written.");
    }
}
