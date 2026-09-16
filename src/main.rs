mod alert;
mod cleaners;
mod config;
mod diag;
mod growth;
mod report;
mod schedule;
mod util;
mod volumes;

use clap::{Parser, Subcommand};
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use util::{expand, home, human, now, state_dir};

#[derive(Parser)]
#[command(
    name = "mac-headroom",
    version,
    about = "Disk pressure diagnostician and cache cleaner for macOS"
)]
struct Cli {
    /// Emit JSON instead of text. Works with every subcommand.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Report disk usage and check for APFS snapshot pinning
    Diagnose,
    /// List the available cleaners and why each is safe
    List,
    /// Run cleaners. Dry run unless --yes is given.
    Clean {
        /// Actually delete. Without this, mac-headroom only reports what it would do.
        #[arg(long)]
        yes: bool,
        /// Run only the named cleaner(s). Repeatable.
        #[arg(long, value_name = "NAME")]
        only: Vec<String>,
    },
    /// Find which directories grew since the last scan
    Growth {
        /// Directory to scan (default: home)
        root: Option<PathBuf>,
        /// How many levels below the root to report on
        #[arg(long, default_value_t = 3)]
        depth: usize,
        /// Ignore changes smaller than this many MB
        #[arg(long, default_value_t = 100)]
        min_mb: u64,
        /// Show at most this many entries
        #[arg(long, default_value_t = 25)]
        top: usize,
    },
    /// Manage the weekly launchd job
    Schedule {
        #[command(subcommand)]
        action: ScheduleCmd,
    },
    /// Manage the config file (user-defined cleaners, disabled built-ins)
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
    /// One-screen overview: disk, config, weekly job, recorded history
    Status,
    /// What else shares this disk: other volumes in the container, and mounted images
    Volumes,
    /// Look at free space and notify if it is low. Cheap enough to run hourly.
    Check {
        /// Notify even if one went out recently
        #[arg(long)]
        force: bool,
    },
    /// Write a self-contained HTML report of the same information and open it
    Report {
        /// Where to write it (default: the state directory)
        #[arg(long, value_name = "FILE")]
        out: Option<PathBuf>,
        /// Write the file without opening a browser
        #[arg(long)]
        no_open: bool,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Write a commented example config (refuses to overwrite)
    Init,
    /// Print the config file path
    Path,
    /// Validate the config and list what it defines
    Check,
}

#[derive(Subcommand)]
enum ScheduleCmd {
    /// Install (or replace) the weekly job
    Install {
        /// Day of the week: mon, tue, ... sun
        #[arg(long, default_value = "mon")]
        weekday: String,
        /// Hour, 0-23
        #[arg(long, default_value_t = 10)]
        hour: u8,
        /// Minute, 0-59
        #[arg(long, default_value_t = 0)]
        minute: u8,
        /// Run only the named cleaner(s). Repeatable. Default: all.
        #[arg(long, value_name = "NAME")]
        only: Vec<String>,
        /// Skip the weekly home growth scan (about a minute)
        #[arg(long)]
        no_growth: bool,
        /// Do not install the hourly low space watch
        #[arg(long)]
        no_watch: bool,
        /// How often the watch looks at free space
        #[arg(long, default_value_t = 60, value_name = "MINUTES")]
        watch_minutes: u64,
    },
    /// Remove the weekly job
    Uninstall,
    /// Show whether the job is installed, its schedule, and the last run
    Status,
    /// Do the weekly routine now: diagnose, growth scan, clean. This is what launchd calls.
    Run {
        #[arg(long, value_name = "NAME")]
        only: Vec<String>,
        #[arg(long)]
        no_growth: bool,
    },
}

fn emit<T: Serialize>(value: &T) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("serialize")
    );
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Diagnose => {
            let Some(r) = diag::report() else {
                eprintln!("could not read diskutil info for /System/Volumes/Data");
                std::process::exit(1);
            };
            if cli.json {
                emit(&r)
            } else {
                diag::print_text(&r)
            }
        }
        Cmd::List => {
            let all = load_cleaners();
            if cli.json {
                emit(&all);
            } else {
                for c in &all {
                    let mut tags = Vec::new();
                    if let Some(p) = &c.skip_if_running {
                        tags.push(format!("skipped while {p} is running"));
                    }
                    if c.source == cleaners::Source::Config {
                        tags.push("from config".into());
                    }
                    if c.disabled {
                        tags.push("disabled in config".into());
                    }
                    let tags = if tags.is_empty() {
                        String::new()
                    } else {
                        format!("  [{}]", tags.join("; "))
                    };
                    println!("{:<20} {}{tags}", c.name, c.summary);
                    println!("{:<20} {}", "", c.why_safe);
                }
            }
        }
        Cmd::Clean { yes, only } => {
            let report = run_clean(cli.json, yes, &only);
            if cli.json {
                emit(&report);
            }
        }
        Cmd::Growth {
            root,
            depth,
            min_mb,
            top,
        } => {
            // With no argument, scan the home folder plus any extra roots from
            // the config, so the breakdown covers more than two thirds of the
            // data volume.
            let roots: Vec<PathBuf> = match root {
                Some(p) => vec![PathBuf::from(expand(&p.to_string_lossy()))],
                None => std::iter::once(home())
                    .chain(config::scan_roots())
                    .collect(),
            };
            let reports: Vec<growth::Report> = roots
                .iter()
                .map(|r| growth::report(r, depth, min_mb << 20, top))
                .collect();
            if cli.json {
                emit(&reports);
            } else {
                for (i, r) in reports.iter().enumerate() {
                    if i > 0 {
                        println!();
                    }
                    growth::print_text(r);
                }
            }
        }
        Cmd::Schedule { action } => match action {
            ScheduleCmd::Install {
                weekday,
                hour,
                minute,
                only,
                no_growth,
                no_watch,
                watch_minutes,
            } => {
                let watch = (!no_watch).then_some(watch_minutes.max(5));
                if let Err(e) = schedule::install(&weekday, hour, minute, &only, !no_growth, watch)
                {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            ScheduleCmd::Uninstall => {
                if let Err(e) = schedule::uninstall() {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            ScheduleCmd::Status => {
                let s = schedule::status();
                if cli.json {
                    emit(&s)
                } else {
                    schedule::print_status(&s)
                }
            }
            ScheduleCmd::Run { only, no_growth } => schedule::run(&only, !no_growth),
        },
        Cmd::Config { action } => match action {
            ConfigCmd::Init => match config::init() {
                Ok(p) => println!(
                    "Wrote {}. Edit it, then run: mac-headroom config check",
                    p.display()
                ),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            },
            ConfigCmd::Path => println!("{}", config::path().display()),
            ConfigCmd::Check => {
                let c = config::check();
                if cli.json {
                    emit(&c)
                } else {
                    config::print_check(&c)
                }
                if !c.problems.is_empty() {
                    std::process::exit(1);
                }
            }
        },
        Cmd::Status => {
            let s = overview();
            if cli.json {
                emit(&s)
            } else {
                print_overview(&s)
            }
        }
        Cmd::Volumes => {
            let v = volumes::gather();
            if cli.json {
                emit(&v)
            } else {
                volumes::print_text(&v)
            }
        }
        Cmd::Check { force } => {
            let Some(c) = alert::check(force) else {
                eprintln!("could not read diskutil info for /System/Volumes/Data");
                std::process::exit(1);
            };
            if cli.json {
                emit(&c)
            } else {
                alert::print_text(&c)
            }
        }
        Cmd::Report { out, no_open } => {
            let data = report::gather();
            let out = out.unwrap_or_else(report::default_path);
            if let Err(e) = report::write(&data, &out) {
                eprintln!("could not write {}: {e}", out.display());
                std::process::exit(1);
            }
            if cli.json {
                emit(&serde_json::json!({ "path": out, "opened": !no_open }));
            } else {
                println!("Wrote {}", out.display());
            }
            if !no_open && !report::open(&out) {
                eprintln!("could not open a browser; open the file by hand");
            }
        }
    }
}

/// Load built-in plus configured cleaners, or exit: a broken config must not
/// silently fall back to "just the built-ins".
fn load_cleaners() -> Vec<cleaners::Cleaner> {
    config::all_cleaners().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(4);
    })
}

#[derive(Serialize)]
struct StateSummary {
    dir: PathBuf,
    diagnose_readings: usize,
    growth_scans: usize,
    audit_entries: usize,
}

#[derive(Serialize)]
struct Overview {
    #[serde(skip_serializing_if = "Option::is_none")]
    disk: Option<DiskNow>,
    config: config::Check,
    cleaners_enabled: Vec<String>,
    schedule: schedule::Status,
    state: StateSummary,
}

#[derive(Serialize)]
struct DiskNow {
    used: u64,
    free: u64,
    total: u64,
}

fn count_lines(p: &std::path::Path) -> usize {
    std::fs::read_to_string(p)
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// Read-only. Unlike `diagnose`, it does not record a history entry.
fn overview() -> Overview {
    let cfg = config::check();
    let cleaners_enabled = if cfg.problems.is_empty() {
        config::all_cleaners()
            .map(|all| {
                all.into_iter()
                    .filter(|c| !c.disabled)
                    .map(|c| c.name)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        vec![]
    };
    let dir = state_dir();
    let growth_scans = std::fs::read_dir(dir.join("growth"))
        .map(|rd| rd.flatten().count())
        .unwrap_or(0);
    Overview {
        disk: diag::disk().map(|d| DiskNow {
            used: d.used,
            free: d.free,
            total: d.total,
        }),
        config: cfg,
        cleaners_enabled,
        schedule: schedule::status(),
        state: StateSummary {
            diagnose_readings: count_lines(&dir.join("history.tsv")),
            growth_scans,
            audit_entries: count_lines(&dir.join("audit.log")),
            dir,
        },
    }
}

fn print_overview(o: &Overview) {
    match &o.disk {
        Some(d) => println!(
            "Disk    {} free of {} ({} used)",
            human(d.free),
            human(d.total),
            human(d.used)
        ),
        None => println!("Disk    unavailable (diskutil failed)"),
    }
    println!();
    config::print_check(&o.config);
    println!();
    println!(
        "Cleaners enabled for `clean`: {}",
        o.cleaners_enabled.join(", ")
    );
    println!();
    print!("Weekly job  ");
    schedule::print_status(&o.schedule);
    println!();
    println!("State   {}", o.state.dir.display());
    println!("  diagnose readings  {}", o.state.diagnose_readings);
    println!("  growth scans       {}", o.state.growth_scans);
    println!("  audit entries      {}", o.state.audit_entries);
}

#[derive(Serialize)]
pub struct CleanReport {
    dry_run: bool,
    free_before: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    free_after: Option<u64>,
    /// Sum over path cleaners only. Command cleaners are not sized.
    pub bytes: u64,
    cleaners: Vec<cleaners::Outcome>,
}

/// Run the selected cleaners. Prints text output unless `json` is set;
/// the caller emits the JSON in that case.
pub fn run_clean(json: bool, apply: bool, only: &[String]) -> CleanReport {
    if apply && std::env::var_os("HEADROOM_NO_DELETE").is_some() {
        eprintln!("refusing: HEADROOM_NO_DELETE is set, so --yes is disabled in this environment");
        std::process::exit(3);
    }
    let all = load_cleaners();
    let selected: Vec<&cleaners::Cleaner> = if only.is_empty() {
        all.iter().filter(|c| !c.disabled).collect()
    } else {
        only.iter()
            .map(|n| {
                cleaners::find(&all, n).unwrap_or_else(|| {
                    eprintln!("unknown cleaner: {n}  (see `mac-headroom list`)");
                    std::process::exit(2);
                })
            })
            .collect()
    };

    if !apply && !json {
        println!("DRY RUN. Nothing will be deleted. Pass --yes to apply.");
    }
    let free_before = diag::disk().map(|d| d.free);

    let mut report = CleanReport {
        dry_run: !apply,
        free_before,
        free_after: None,
        bytes: 0,
        cleaners: Vec::new(),
    };
    for c in selected {
        let o = cleaners::run(c, apply);
        if !json {
            cleaners::print_text(c, &o);
        }
        report.bytes += o.bytes;
        report.cleaners.push(o);
    }
    if apply {
        report.free_after = diag::disk().map(|d| d.free);
        audit(&report);
    }

    if !json {
        println!();
        match (report.free_before, report.free_after) {
            (Some(b), Some(a)) => println!("Done. Free space {} -> {}", human(b), human(a)),
            _ if !apply => println!(
                "Would reclaim at least {} from path cleaners. Command cleaners are not sized in a dry run.",
                human(report.bytes)
            ),
            _ => println!("Done."),
        }
    }
    report
}

/// Every real deletion is logged, so a missing file can be traced back or ruled out.
fn audit(r: &CleanReport) {
    let path = state_dir().join("audit.log");
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let ts = now();
    for o in &r.cleaners {
        for t in &o.targets {
            // A target a filter kept was never touched. Recording it as a
            // deletion makes the one record meant to be trustworthy a lie.
            if t.kept.is_some() {
                continue;
            }
            let result = t.error.as_deref().unwrap_or("deleted");
            let _ = writeln!(
                f,
                "{ts}\t{}\t{result}\t{}\t{}",
                o.name,
                t.bytes,
                t.path.display()
            );
        }
        if let Some(cmd) = &o.command {
            let status = serde_json::to_string(&o.status).unwrap_or_default();
            let _ = writeln!(
                f,
                "{ts}\t{}\t{}\t0\t{cmd}",
                o.name,
                status.trim_matches('"')
            );
        }
    }
}
