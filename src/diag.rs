//! The diagnostic that matters: is the missing space files, or is it pinned by
//! APFS snapshots? Snapshot-pinned blocks are invisible to `du`, and don't count
//! toward "Volume Used" either. The tell is that used and free stop moving
//! together between runs.

use crate::util::{ago, human, now, signed, state_dir, stdout_of};
use serde::Serialize;
use std::fs;
use std::io::Write;

/// Decimal, to match the sizes `human` prints beside these thresholds.
const GB: i64 = 1_000_000_000;

pub struct Disk {
    pub used: u64,
    pub free: u64,
    pub total: u64,
    /// The whole drive the container sits on. Larger than `total`, because the
    /// recovery and firmware partitions sit outside the container and no
    /// volume can use them. The report shows the difference rather than
    /// leaving someone to wonder where it went.
    pub media: Option<u64>,
}

fn bytes_field(diskutil_out: &str, label: &str) -> Option<u64> {
    // Lines look like: "   Volume Used Space:   174.8 GB (174847209472 Bytes) (exactly ...)"
    let line = diskutil_out.lines().find(|l| l.contains(label))?;
    let start = line.find('(')? + 1;
    let end = line[start..].find(" Bytes")? + start;
    line[start..end].trim().parse().ok()
}

/// The drive holding the container, measured whole. `diskutil info` names the
/// container's physical store, `disk0s2`; the drive is that without the slice.
/// A container spread over several stores has no single drive to point at, so
/// this reports nothing rather than passing one store off as the disk.
fn media_bytes(info: &str) -> Option<u64> {
    let mut stores = info
        .lines()
        .filter_map(|l| l.split_once("APFS Physical Store:"))
        .map(|(_, v)| v.trim());
    let store = stores.next()?;
    if stores.next().is_some() {
        return None;
    }
    let digits: String = store
        .strip_prefix("disk")?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        return None;
    }
    let whole = format!("disk{digits}");
    bytes_field(&stdout_of("diskutil", &["info", &whole]), "Disk Size")
}

pub fn disk() -> Option<Disk> {
    let info = stdout_of("diskutil", &["info", "/System/Volumes/Data"]);
    let total = bytes_field(&info, "Container Total Space")?;
    Some(Disk {
        used: bytes_field(&info, "Volume Used Space")?,
        free: bytes_field(&info, "Container Free Space")?,
        total,
        // A drive smaller than the container it holds means the parse is
        // wrong, and a wrong number explained confidently is worse than none.
        media: media_bytes(&info).filter(|m| *m >= total),
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

/// One row of `history.tsv`.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub at: u64,
    pub used: u64,
    pub free: u64,
}

/// How old a reading has to be before it is worth comparing against.
const BASELINE_AGE_SECS: u64 = 24 * 3600;

/// `check` runs hourly, and something else may call it more often. A reading
/// closer than this to the last one adds a row and no information.
pub const CHECK_READING_GAP_SECS: u64 = 50 * 60;

fn history_path() -> std::path::PathBuf {
    state_dir().join("history.tsv")
}

/// Every reading on record, oldest first. Lines that do not parse are skipped.
pub fn history() -> Vec<Reading> {
    parse_history(&fs::read_to_string(history_path()).unwrap_or_default())
}

fn parse_history(text: &str) -> Vec<Reading> {
    text.lines()
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

/// The reading to compare the present against: the newest one at least a day
/// old, or the oldest there is while the record is younger than that.
///
/// It used to be simply the last row, which was right while a row was written
/// once a week. With a row an hour it would mean comparing against an hour
/// ago, where nothing has moved far enough to cross a threshold, and the
/// diagnosis would go quiet exactly when readings became plentiful.
pub fn baseline(rows: &[Reading], now: u64) -> Option<Reading> {
    rows.iter()
        .rev()
        .find(|r| now.saturating_sub(r.at) >= BASELINE_AGE_SECS)
        .or_else(|| rows.first())
        .copied()
}

/// The last week of readings and what they add up to, for anything that
/// shows the trend without wanting to do the arithmetic itself.
#[derive(Serialize, Debug, PartialEq)]
pub struct Trend {
    /// Readings from the last seven days, oldest first.
    pub readings: Vec<Reading>,
    /// When the oldest of them was taken.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// Free space now, less free space at `since`. Negative means less free
    /// than a week ago. Absent until the record spans a day, because two
    /// readings an hour apart say nothing about the week.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_change: Option<i64>,
    /// At the week's rate, days until nothing is free. Only when free space
    /// is falling and the answer is under a year: it is an extrapolation of
    /// one week, and past that it would be a number with no meaning.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_in_days: Option<f64>,
}

pub const TREND_WINDOW_SECS: u64 = 7 * 24 * 3600;
const TREND_MIN_SPAN_SECS: u64 = 24 * 3600;

pub fn trend(rows: &[Reading], now: u64, free_now: u64) -> Trend {
    let readings: Vec<Reading> = rows
        .iter()
        .filter(|r| now.saturating_sub(r.at) <= TREND_WINDOW_SECS)
        .copied()
        .collect();
    let first = readings.first().copied();
    let spans_a_day = first.is_some_and(|f| now.saturating_sub(f.at) >= TREND_MIN_SPAN_SECS);
    let free_change = if spans_a_day {
        first.map(|f| free_now as i64 - f.free as i64)
    } else {
        None
    };
    let full_in_days = match (first, free_change) {
        (Some(f), Some(change)) if change < 0 => {
            let per_sec = (-change) as f64 / (now - f.at) as f64;
            let days = free_now as f64 / per_sec / 86_400.0;
            (days < 365.0).then_some((days * 10.0).round() / 10.0)
        }
        _ => None,
    };
    Trend {
        readings,
        since: first.map(|f| f.at),
        free_change,
        full_in_days,
    }
}

/// One line for the terminal: what the week's readings say.
pub fn trend_sentence(t: &Trend) -> String {
    match (t.free_change, t.full_in_days) {
        (None, _) => match t.readings.len() {
            0 => "no readings yet".to_string(),
            n => format!(
                "{n} reading{} so far; a day of them is needed",
                if n == 1 { "" } else { "s" }
            ),
        },
        (Some(change), Some(days)) => format!(
            "{} free this week; full in about {days:.0} days at this rate",
            signed(change)
        ),
        (Some(change), None) => format!("{} free this week", signed(change)),
    }
}

fn append_reading(d: &Disk, at: u64) {
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(history_path())
    {
        let _ = writeln!(f, "{}\t{}\t{}", at, d.used, d.free);
    }
}

/// Whether a reading taken now would be far enough from the last one.
fn reading_is_due(rows: &[Reading], now: u64, min_gap: u64) -> bool {
    rows.last()
        .is_none_or(|last| now.saturating_sub(last.at) >= min_gap)
}

/// For callers that run often and measure the disk anyway. Returns whether a
/// row was written.
pub fn record_if_due(d: &Disk, min_gap: u64) -> bool {
    let at = now();
    let due = reading_is_due(&history(), at, min_gap);
    if due {
        append_reading(d, at);
    }
    due
}

/// What `record` did.
#[derive(Serialize)]
pub struct Recorded {
    /// False when the last reading was too recent for another to add anything.
    pub recorded: bool,
    pub used: u64,
    pub free: u64,
}

/// Measure the disk and add a reading, if one is due. It writes nothing else,
/// compares nothing and notifies nobody: `check` does all three, and a caller
/// with its own low-space warning would otherwise send two.
pub fn record() -> Option<Recorded> {
    let d = disk()?;
    Some(Recorded {
        recorded: record_if_due(&d, CHECK_READING_GAP_SECS),
        used: d.used,
        free: d.free,
    })
}

/// Local APFS snapshots. A staged macOS update leaves `com.apple.os.update-*`
/// entries here, and those pin space that `du` cannot see.
pub fn snapshots() -> Vec<String> {
    stdout_of("tmutil", &["listlocalsnapshots", "/"])
        .lines()
        .filter(|l| l.starts_with("com.apple"))
        .map(str::to_string)
        .collect()
}

pub fn report() -> Option<Report> {
    let d = disk()?;
    let snapshots = snapshots();
    let update_snapshot_pinned = snapshots.iter().any(|n| n.contains("com.apple.os.update"));

    let at = now();
    let since_last = baseline(&history(), at).map(|b| {
        let used = d.used as i64 - b.used as i64;
        let free = d.free as i64 - b.free as i64;
        Delta {
            previous_at: b.at,
            used,
            free,
            unaccounted: -free - used,
        }
    });
    append_reading(&d, at);

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
            "Since the reading {}: used {}, free {}",
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

    fn at(at: u64) -> Reading {
        Reading {
            at,
            used: at,
            free: at,
        }
    }

    #[test]
    fn baseline_is_chosen_by_age_not_position() {
        let day = BASELINE_AGE_SECS;
        let now = 10 * day;
        // A week-old reading, one just over a day old, then a run of hourly ones.
        let rows = [
            at(3 * day),
            at(now - day - 60),
            at(now - 7200),
            at(now - 3600),
        ];
        assert_eq!(baseline(&rows, now), Some(at(now - day - 60)));
        // Weekly rows only: the last row is the baseline, as it always was.
        assert_eq!(baseline(&rows[..1], now), Some(at(3 * day)));
    }

    #[test]
    fn a_young_record_compares_against_its_first_reading() {
        let now = 1_000_000;
        let rows = [at(now - 7200), at(now - 3600), at(now - 60)];
        assert_eq!(baseline(&rows, now), Some(at(now - 7200)));
        assert_eq!(baseline(&[], now), None);
    }

    #[test]
    fn a_reading_is_due_only_once_the_gap_has_passed() {
        let now = 1_000_000;
        assert!(reading_is_due(&[], now, 3000));
        assert!(reading_is_due(&[at(now - 3000)], now, 3000));
        assert!(!reading_is_due(&[at(now - 2999)], now, 3000));
        // A clock that went backwards must not stop readings for good... but
        // it must not write one a second either.
        assert!(!reading_is_due(&[at(now + 50)], now, 3000));
    }

    fn free_at(at: u64, free: u64) -> Reading {
        Reading {
            at,
            used: 500 * 1_000_000_000 - free,
            free,
        }
    }

    #[test]
    fn trend_keeps_a_week_and_says_how_free_space_moved() {
        let day = 24 * 3600;
        let gb = 1_000_000_000u64;
        let now = 100 * day;
        let rows = vec![
            free_at(now - 10 * day, 90 * gb),
            free_at(now - 6 * day, 60 * gb),
            free_at(now - 3 * day, 55 * gb),
            free_at(now - 3600, 51 * gb),
        ];
        let t = trend(&rows, now, 50 * gb);
        assert_eq!(t.readings.len(), 3, "the ten-day-old reading is out");
        assert_eq!(t.since, Some(now - 6 * day));
        assert_eq!(t.free_change, Some(-10 * gb as i64));
        // 10 GB in 6 days, 50 GB left: 30 days.
        assert_eq!(t.full_in_days, Some(30.0));
        assert_eq!(
            trend_sentence(&t),
            "-10 GB free this week; full in about 30 days at this rate"
        );
    }

    #[test]
    fn trend_says_nothing_until_the_record_spans_a_day() {
        let gb = 1_000_000_000u64;
        let now = 1_000_000;
        let rows = vec![free_at(now - 7200, 60 * gb), free_at(now - 3600, 55 * gb)];
        let t = trend(&rows, now, 50 * gb);
        assert_eq!(t.readings.len(), 2);
        assert_eq!(t.free_change, None);
        assert_eq!(t.full_in_days, None);
        assert_eq!(
            trend_sentence(&t),
            "2 readings so far; a day of them is needed"
        );
        assert_eq!(trend_sentence(&trend(&[], now, 50 * gb)), "no readings yet");
    }

    #[test]
    fn trend_has_no_full_date_when_free_space_grew_or_barely_moves() {
        let day = 24 * 3600;
        let gb = 1_000_000_000u64;
        let now = 100 * day;
        let grew = trend(&[free_at(now - 2 * day, 40 * gb)], now, 50 * gb);
        assert_eq!(grew.free_change, Some(10 * gb as i64));
        assert_eq!(grew.full_in_days, None);
        assert_eq!(trend_sentence(&grew), "+10 GB free this week");
        // 1 MB a week off 50 GB is centuries away, and not worth a number.
        let crawl = trend(&[free_at(now - 7 * day, 50 * gb + 1_000_000)], now, 50 * gb);
        assert_eq!(crawl.full_in_days, None);
    }

    #[test]
    fn history_skips_lines_that_do_not_parse() {
        let rows = parse_history("1\t2\t3\n\nnot a row\n4\t5\n7\t8\t9\n");
        assert_eq!(rows.iter().map(|r| r.at).collect::<Vec<_>>(), vec![1, 7]);
    }
}
