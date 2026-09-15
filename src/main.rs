mod cleaners;
mod diag;
mod growth;
mod util;

use clap::{Parser, Subcommand};
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use util::{expand, home, human, now, state_dir};

#[derive(Parser)]
#[command(name = "mac-headroom", version, about = "Disk pressure diagnostician and cache cleaner for macOS")]
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
}

fn emit<T: Serialize>(value: &T) {
    println!("{}", serde_json::to_string_pretty(value).expect("serialize"));
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Diagnose => {
            let Some(r) = diag::report() else {
                eprintln!("could not read diskutil info for /System/Volumes/Data");
                std::process::exit(1);
            };
            if cli.json { emit(&r) } else { diag::print_text(&r) }
        }
        Cmd::List => {
            if cli.json {
                emit(&cleaners::CLEANERS);
            } else {
                for c in cleaners::CLEANERS {
                    let skip = c
                        .skip_if_running
                        .map(|p| format!("  [skipped while {p} is running]"))
                        .unwrap_or_default();
                    println!("{:<20} {}{skip}", c.name, c.summary);
                    println!("{:<20} {}", "", c.why_safe);
                }
            }
        }
        Cmd::Clean { yes, only } => clean(cli.json, yes, &only),
        Cmd::Growth { root, depth, min_mb, top } => {
            let root = root
                .map(|p| PathBuf::from(expand(&p.to_string_lossy())))
                .unwrap_or_else(home);
            let r = growth::report(&root, depth, min_mb << 20, top);
            if cli.json { emit(&r) } else { growth::print_text(&r) }
        }
    }
}

#[derive(Serialize)]
struct CleanReport {
    dry_run: bool,
    free_before: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    free_after: Option<u64>,
    /// Sum over path cleaners only. Command cleaners are not sized.
    bytes: u64,
    cleaners: Vec<cleaners::Outcome>,
}

fn clean(json: bool, apply: bool, only: &[String]) {
    if apply && std::env::var_os("HEADROOM_NO_DELETE").is_some() {
        eprintln!("refusing: HEADROOM_NO_DELETE is set, so --yes is disabled in this environment");
        std::process::exit(3);
    }
    let selected: Vec<&cleaners::Cleaner> = if only.is_empty() {
        cleaners::CLEANERS.iter().collect()
    } else {
        only.iter()
            .map(|n| {
                cleaners::find(n).unwrap_or_else(|| {
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

    let mut report = CleanReport { dry_run: !apply, free_before, free_after: None, bytes: 0, cleaners: Vec::new() };
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

    if json {
        emit(&report);
        return;
    }
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

/// Every real deletion is logged, so a missing file can be traced back or ruled out.
fn audit(r: &CleanReport) {
    let path = state_dir().join("audit.log");
    let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else { return };
    let ts = now();
    for o in &r.cleaners {
        for t in &o.targets {
            let result = t.error.as_deref().unwrap_or("deleted");
            let _ = writeln!(f, "{ts}\t{}\t{result}\t{}\t{}", o.name, t.bytes, t.path.display());
        }
        if let Some(cmd) = &o.command {
            let _ = writeln!(f, "{ts}\t{}\t{}\t0\t{cmd}", o.name, serde_json::to_string(&o.status).unwrap_or_default().trim_matches('"'));
        }
    }
}
