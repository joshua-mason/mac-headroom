use crate::util::{disk_usage, expand, human, is_running, on_path, tilde};
use serde::Serialize;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Action {
    /// Delete every filesystem entry matched by these globs.
    Paths(&'static [&'static str]),
    /// Run an external tool's own cache-clearing command.
    Cmd(&'static [&'static str]),
}

#[derive(Serialize)]
pub struct Cleaner {
    pub name: &'static str,
    pub summary: &'static str,
    /// The reason this is safe to delete. Every cleaner must justify itself.
    pub why_safe: &'static str,
    /// Skip the cleaner when this process is running (exact name, as `pgrep -x`).
    pub skip_if_running: Option<&'static str>,
    pub action: Action,
}

pub const CLEANERS: &[Cleaner] = &[
    Cleaner {
        name: "xcode-derived-data",
        summary: "Xcode build intermediates",
        why_safe: "Rebuilt on the next build. Skipped while Xcode is open so a live build is not disrupted.",
        skip_if_running: Some("Xcode"),
        action: Action::Paths(&["~/Library/Developer/Xcode/DerivedData/*"]),
    },
    Cleaner {
        name: "updater-leftovers",
        summary: "Downloaded app updates that were already applied",
        why_safe: "Electron/Squirrel updaters leave the installed update on disk. The app re-downloads if it ever needs it.",
        skip_if_running: None,
        action: Action::Paths(&[
            "~/Library/Caches/com.microsoft.VSCode.ShipIt/*",
            "~/Library/Caches/com.tinyspeck.slackmacgap.ShipIt/*",
            "~/Library/Caches/electron/*",
        ]),
    },
    Cleaner {
        name: "language-caches",
        summary: "pip, SwiftPM, node-gyp and TypeScript download caches",
        why_safe: "Pure download caches. The next install fetches what it needs again.",
        skip_if_running: None,
        action: Action::Paths(&[
            "~/Library/Caches/pip/*",
            "~/Library/Caches/org.swift.swiftpm/*",
            "~/Library/Caches/node-gyp/*",
            "~/Library/Caches/typescript/*",
        ]),
    },
    Cleaner {
        name: "spotify-cache",
        summary: "Spotify streaming cache",
        why_safe: "Streamed audio only. Playlists and downloads for offline live elsewhere. Skipped while Spotify is open.",
        skip_if_running: Some("Spotify"),
        action: Action::Paths(&["~/Library/Caches/com.spotify.client/*"]),
    },
    Cleaner {
        name: "chrome-cache",
        summary: "Chrome HTTP, code and GPU caches (profile data untouched)",
        why_safe: "Only ~/Library/Caches/Google/Chrome. Cookies, history, IndexedDB and Service Workers live under Application Support and are never touched. Skipped while Chrome is open.",
        skip_if_running: Some("Google Chrome"),
        action: Action::Paths(&[
            "~/Library/Caches/Google/Chrome/*/Cache",
            "~/Library/Caches/Google/Chrome/*/Code Cache",
            "~/Library/Caches/Google/Chrome/*/GPUCache",
        ]),
    },
    Cleaner {
        name: "npm-cache",
        summary: "npm package cache",
        why_safe: "npm's own cache clean. Packages re-download on the next install.",
        skip_if_running: None,
        action: Action::Cmd(&["npm", "cache", "clean", "--force"]),
    },
    Cleaner {
        name: "pnpm-store",
        summary: "pnpm store (unreferenced packages only)",
        why_safe: "pnpm's own prune. It removes only packages no project references.",
        skip_if_running: None,
        action: Action::Cmd(&["pnpm", "store", "prune"]),
    },
    Cleaner {
        name: "uv-cache",
        summary: "uv Python package cache",
        why_safe: "uv's own cache clean. Wheels re-download on the next sync.",
        skip_if_running: None,
        action: Action::Cmd(&["uv", "cache", "clean"]),
    },
    Cleaner {
        name: "go-build-cache",
        summary: "Go build cache",
        why_safe: "Go's own clean. The next build recompiles.",
        skip_if_running: None,
        action: Action::Cmd(&["go", "clean", "-cache"]),
    },
    Cleaner {
        name: "homebrew",
        summary: "Homebrew downloads and old formula versions",
        why_safe: "brew cleanup with --prune=all. Keeps every installed formula, removes only downloads and superseded versions.",
        skip_if_running: None,
        action: Action::Cmd(&["brew", "cleanup", "-s", "--prune=all"]),
    },
];

pub fn find(name: &str) -> Option<&'static Cleaner> {
    CLEANERS.iter().find(|c| c.name == name)
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct Outcome {
    pub name: &'static str,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub targets: Vec<Target>,
    /// Bytes reclaimed, or that would be. Always 0 for command cleaners.
    pub bytes: u64,
}

fn matches(globs: &[&str]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = globs
        .iter()
        .flat_map(|g| glob::glob(&expand(g)).into_iter().flatten().flatten())
        .collect();
    out.sort();
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
        name: c.name,
        status: Status::Nothing,
        reason: None,
        command: None,
        targets: Vec::new(),
        bytes: 0,
    };
    if let Some(proc_name) = c.skip_if_running {
        if is_running(proc_name) {
            out.status = Status::Skipped;
            out.reason = Some(format!("{proc_name} is running"));
            return out;
        }
    }
    match c.action {
        Action::Paths(globs) => {
            let paths = matches(globs);
            if paths.is_empty() {
                return out;
            }
            for path in paths {
                let bytes = disk_usage(&path);
                let error = if apply {
                    remove(&path).err().map(|e| e.to_string())
                } else {
                    None
                };
                if error.is_none() {
                    out.bytes += bytes;
                }
                out.targets.push(Target { path, bytes, error });
            }
            out.status = if apply {
                Status::Cleared
            } else {
                Status::WouldClear
            };
        }
        Action::Cmd(argv) => {
            out.command = Some(argv.join(" "));
            if !on_path(argv[0]) {
                out.status = Status::Skipped;
                out.reason = Some(format!("{} is not installed", argv[0]));
                return out;
            }
            if !apply {
                out.status = Status::WouldRun;
                return out;
            }
            let status = Command::new(argv[0])
                .args(&argv[1..])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            out.status = match status {
                Ok(s) if s.success() => Status::Ran,
                _ => Status::Failed,
            };
        }
    }
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
        Status::WouldClear | Status::Cleared => {
            for t in &o.targets {
                println!("  {:>9}  {}", human(t.bytes), tilde(&t.path));
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
