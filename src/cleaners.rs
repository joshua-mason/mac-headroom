use crate::util::{ago, disk_usage, expand, home, human, is_running, now, on_path, tilde};
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

/// How `keep_newest` decides which matches are copies of the same thing.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum GroupBy {
    /// The name with a trailing version cut off, so that
    /// `anthropic.claude-code-2.1.276-darwin-arm64` and the five older folders
    /// beside it are six copies of one extension rather than six unrelated
    /// things. A name with no version in it is left in a group of its own.
    NameBeforeVersion,
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
    /// Narrows what `keep_newest` compares. Without it, matches compete with
    /// everything else in their directory, which is wrong for any tool that
    /// puts every version of every component side by side in one folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_by: Option<GroupBy>,
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
    /// A command that prints the directory a command cleaner empties. Measuring
    /// it before and after is the only way to know what a tool's own clean
    /// command actually freed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measure: Vec<String>,
    /// Built-in only: files are found by asking an application's own database
    /// what it still references, rather than by matching a path.
    #[serde(skip)]
    pub orphans: Option<&'static crate::orphans::OrphanSpec>,
    /// A name someone who has never heard of the underlying tool would
    /// recognise. The report leads with this; `name` stays the stable handle.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// One or two plain sentences: what this is and why removing it is fine.
    /// `why_safe` carries the precise reasoning for people who want it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plain: String,
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
        group_by: None,
        older_than_days: None,
        source: Source::Builtin,
        disabled: false,
        opt_in: false,
        measure: vec![],
        orphans: None,
        title: String::new(),
        plain: String::new(),
    }
}

impl Cleaner {
    fn with_plain(mut self, title: &str, plain: &str) -> Self {
        self.title = title.into();
        self.plain = plain.into();
        self
    }
}

fn cmd(name: &str, summary: &str, why_safe: &str, argv: &[&str], measure: &[&str]) -> Cleaner {
    Cleaner {
        command: argv.iter().map(|s| s.to_string()).collect(),
        measure: measure.iter().map(|s| s.to_string()).collect(),
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
        )
        .with_plain(
            "Xcode build leftovers",
            "Temporary files Xcode makes while it builds apps. Xcode recreates them the next time you build.",
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
        )
        .with_plain(
            "Old app update downloads",
            "Copies of updates that apps like VS Code and Slack have already installed. Nothing uses them any more.",
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
        )
        .with_plain(
            "Downloaded programming packages",
            "Packages kept by tools such as Python's pip and Swift. They download again if a project needs them.",
        ),
        paths(
            "spotify-cache",
            "Spotify streaming cache",
            "Streamed audio only. Playlists and downloads for offline live elsewhere. Skipped while Spotify is open.",
            Some("Spotify"),
            &["~/Library/Caches/com.spotify.client/*"],
        )
        .with_plain(
            "Spotify's music cache",
            "Songs Spotify saved while you streamed. Your playlists and any music you downloaded for offline listening are not affected.",
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
        )
        .with_plain(
            "Chrome's web cache",
            "Images and files Chrome saved to load websites faster. Your bookmarks, passwords, history and logins are not touched.",
        ),
        paths(
            "terraform-plugins",
            "Terraform's shared provider plugin cache",
            "A download cache kept so repeated `terraform init` runs are fast. A project that used it may hold links into it, so the next init in that project downloads again. Nothing is lost but the download.",
            None,
            &["~/.terraform.d/plugin-cache/*"],
        )
        .with_plain(
            "Downloaded Terraform providers",
            "Copies of the plugins Terraform downloads so it can talk to cloud services. It fetches them again the next time a project needs one.",
        ),
        paths(
            "playwright-browsers",
            "Browsers the Playwright test tool downloaded",
            "Playwright keeps its own copies of Chromium, Firefox and WebKit, separate from any browser you use. `npx playwright install` downloads them again.",
            None,
            &["~/Library/Caches/ms-playwright/*"],
        )
        .with_plain(
            "Test browsers",
            "Private copies of browsers that the Playwright testing tool downloads. It downloads them again when a test needs one.",
        ),
        Cleaner {
            keep_newest: Some(1),
            group_by: Some(GroupBy::NameBeforeVersion),
            ..paths(
                "vscode-old-extensions",
                "Superseded copies of VS Code extensions",
                "VS Code installs each extension update beside the last and runs the newest, so only the newest of each is kept here. An extension that is ever needed again is re-downloaded. Skipped while VS Code is open.",
                Some("Code"),
                &["~/.vscode/extensions/*"],
            )
        }
        .with_plain(
            "Old versions of VS Code extensions",
            "VS Code keeps the previous copy of an add-on after updating it. Only the newest of each is used.",
        ),
        cmd(
            "npm-cache",
            "npm package cache",
            "npm's own cache clean. Packages re-download on the next install.",
            &["npm", "cache", "clean", "--force"],
            &["npm", "config", "get", "cache"],
        )
        .with_plain(
            "Downloaded JavaScript packages (npm)",
            "Copies of code libraries that npm downloaded for your projects. They download again whenever a project needs them.",
        ),
        cmd(
            "pnpm-store",
            "pnpm store (unreferenced packages only)",
            "pnpm's own prune. It removes only packages no project references.",
            &["pnpm", "store", "prune"],
            &["pnpm", "store", "path"],
        )
        .with_plain(
            "Unused JavaScript packages (pnpm)",
            "Packages that none of your projects use any more. Anything still in use is kept.",
        ),
        cmd(
            "uv-cache",
            "uv Python package cache",
            "uv's own cache clean. Wheels re-download on the next sync.",
            &["uv", "cache", "clean"],
            &["uv", "cache", "dir"],
        )
        .with_plain(
            "Downloaded Python packages (uv)",
            "Copies of Python libraries that uv downloaded. They download again whenever a project needs them.",
        ),
        cmd(
            "go-build-cache",
            "Go build cache",
            "Go's own clean. The next build recompiles.",
            &["go", "clean", "-cache"],
            &["go", "env", "GOCACHE"],
        )
        .with_plain(
            "Go build cache",
            "Compiled pieces Go keeps so builds are quicker. Go rebuilds them when it needs them.",
        ),
        // Actions on things under "worth a look". Opt in, so a plain `clean`
        // never touches them: each is a deliberate choice someone makes once,
        // not part of a weekly routine. Being cleaners is what gives them the
        // dry run, the skip-while-running check and the audit log for free.
        Cleaner {
            opt_in: true,
            ..paths(
                "claude-vm-bundles",
                "Claude Desktop's sandbox virtual machine image",
                "The disk image Claude Desktop's sandbox runs inside, rebuilt the next time the sandbox is used. Anything sitting in an unfinished sandbox session goes with it, which is why this is offered as a deliberate action and never as part of a routine clean. Skipped while Claude is open, since the image is then in use.",
                Some("Claude"),
                &["~/Library/Application Support/Claude/vm_bundles/*"],
            )
        }
        .with_plain(
            "Claude Desktop's sandbox image",
            "The private disk Claude Desktop uses to run code away from your own files. It makes a new one the next time it needs one.",
        ),
        Cleaner {
            opt_in: true,
            ..paths(
                "xcode-device-support",
                "Debug symbols Xcode copied from connected iPhones and iPads",
                "Xcode copies these from a device the first time it sees that iOS version, and copies them again if it needs them. Nothing in them is yours. Skipped while Xcode is open.",
                Some("Xcode"),
                &["~/Library/Developer/Xcode/iOS DeviceSupport/*"],
            )
        }
        .with_plain(
            "Xcode device support files",
            "Debugging files Xcode copied from iPhones you have plugged in. It copies them again if it needs them.",
        ),
        Cleaner {
            opt_in: true,
            skip_if_running: Some("Xcode".into()),
            ..cmd(
                "simulators-unavailable",
                "Simulators whose iOS version is no longer installed",
                "simctl's own delete of unavailable devices: simulators whose runtime this Mac no longer has, so nothing can boot them. Simulators for the runtimes you do have are untouched, and Xcode makes fresh ones on demand. Skipped while Xcode is open.",
                &["xcrun", "simctl", "delete", "unavailable"],
                &[],
            )
        }
        .with_plain(
            "Simulators that cannot be booted",
            "Simulated devices left behind for iOS versions this Mac no longer has installed.",
        ),
        orphan_cleaner(
            "whatsapp-orphans",
            "WhatsApp media it no longer has any record of",
            "Every file is checked against WhatsApp's own database, including its write-ahead log, and only files that nothing in it references are removed. Re-linking the Mac as a device replaces that database, which strands everything downloaded under the old one: the app cannot see those files, so its storage screen never frees them. Anything still on your phone or within WhatsApp's retention re-downloads when you scroll back. Skipped while WhatsApp is open, and refused outright if the database cannot be read.",
            Some("WhatsApp"),
            &crate::orphans::WHATSAPP,
        )
        .with_plain(
            "WhatsApp files it has forgotten",
            "Photos and videos WhatsApp downloaded and then lost track of, usually after you logged in again. WhatsApp itself cannot see or remove them.",
        ),
        cmd(
            "homebrew",
            "Homebrew downloads and old formula versions",
            "brew cleanup with --prune=all. Keeps every installed formula, removes only downloads and superseded versions.",
            &["brew", "cleanup", "-s", "--prune=all"],
            &["brew", "--cache"],
        )
        .with_plain(
            "Homebrew leftovers",
            "Old versions and installer downloads left by Homebrew, the tool that installs command line programs. The programs you have installed stay.",
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
        if self.command.is_empty() && !self.measure.is_empty() {
            return Err(format!("{n}: measure only applies to a command cleaner"));
        }
        if self.group_by.is_some() && self.keep_newest.is_none() {
            return Err(format!(
                "{n}: group_by says which matches compete, so it needs keep_newest"
            ));
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
    /// Bytes reclaimed, or that would be. For a command cleaner this is only
    /// meaningful when `measured` is true.
    pub bytes: u64,
    /// Whether `bytes` for a command cleaner comes from a measurement. A tool's
    /// own clean command reports nothing we can trust, so without a measurement
    /// the honest figure is "unknown", not zero.
    pub measured: bool,
    /// For a command cleaner in a dry run: how big its cache is now. An upper
    /// bound on what running it could free, not a promise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_bytes: Option<u64>,
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

/// The part of a name before a trailing version: the `-` that is followed by a
/// digit is where a version starts, and everything from there is the version
/// and whatever the tool appends to it (`-darwin-arm64` and the like).
/// None when there is no version to cut, which leaves that match in a group of
/// its own. A thing we cannot classify never loses a comparison it was not in.
fn name_before_version(name: &str) -> Option<&str> {
    let b = name.as_bytes();
    (1..b.len().saturating_sub(1))
        .find(|&i| b[i] == b'-' && b[i + 1].is_ascii_digit())
        .map(|i| &name[..i])
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
        // Grouped by directory, and by name as well when the cleaner says the
        // directory holds many versions of many things.
        let mut groups: BTreeMap<(PathBuf, String), Vec<(usize, u64)>> = BTreeMap::new();
        for (i, (p, kept)) in out.iter().enumerate() {
            if kept.is_none() {
                let parent = p.parent().map(Path::to_path_buf).unwrap_or_default();
                let key = match c.group_by {
                    None => String::new(),
                    Some(GroupBy::NameBeforeVersion) => {
                        let name = p.file_name().unwrap_or_default().to_string_lossy();
                        name_before_version(&name).unwrap_or(&name).to_string()
                    }
                };
                groups
                    .entry((parent, key))
                    .or_default()
                    .push((i, own_mtime(p)));
            }
        }
        for ((_, key), group) in groups.iter_mut() {
            group.sort_by_key(|&(_, mtime)| std::cmp::Reverse(mtime));
            for &(i, _) in group.iter().take(n) {
                out[i].1 = Some(match (c.group_by.is_some(), n) {
                    (false, 1) => "newest in its directory".to_string(),
                    (false, _) => format!("among the {n} newest in its directory"),
                    (true, 1) => format!("newest copy of {key}"),
                    (true, _) => format!("among the {n} newest copies of {key}"),
                });
            }
        }
    }
    out
}

/// What a cleaner could free right now, found without deleting anything.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Estimate {
    pub name: String,
    pub title: String,
    pub plain: String,
    /// None when it cannot be measured, for instance a tool that is not installed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// True for a tool's own clean command: this is the size of its cache, and
    /// the command may free less than all of it.
    pub upper_bound: bool,
    /// An app that has to be closed before this can run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub opt_in: bool,
}

/// Size what a cleaner would remove, reading only. Unlike a dry run, an app
/// being open does not hide the figure; it is reported as what to close first.
pub fn estimate(c: &Cleaner) -> Estimate {
    let mut e = Estimate {
        name: c.name.clone(),
        title: if c.title.is_empty() {
            c.summary.clone()
        } else {
            c.title.clone()
        },
        plain: if c.plain.is_empty() {
            c.why_safe.clone()
        } else {
            c.plain.clone()
        },
        bytes: None,
        upper_bound: false,
        blocked_by: c.skip_if_running.clone().filter(|p| is_running(p)),
        note: None,
        opt_in: c.opt_in,
    };
    if c.orphans.is_some() {
        e.note = Some("reported under things worth a look".into());
        return e;
    }
    if !c.command.is_empty() {
        if !on_path(&c.command[0]) {
            e.note = Some(format!("{} is not installed", c.command[0]));
            return e;
        }
        if let Some(dir) = measure_dir(&c.measure) {
            e.bytes = Some(disk_usage(&dir));
            e.upper_bound = true;
        }
        return e;
    }
    e.bytes = Some(
        candidates(c)
            .into_iter()
            .filter(|(_, kept)| kept.is_none())
            .map(|(p, _)| disk_usage(&p))
            .sum(),
    );
    e
}

/// Ask a tool where its cache lives. Anything that is not an existing absolute
/// directory is treated as "cannot measure", never as an empty cache.
fn measure_dir(argv: &[String]) -> Option<PathBuf> {
    let (bin, args) = argv.split_first()?;
    if !on_path(bin) {
        return None;
    }
    let out = Command::new(bin)
        .args(args)
        .current_dir(home())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())?
        .to_string();
    let dir = PathBuf::from(expand(&line));
    (dir.is_absolute() && dir.is_dir()).then_some(dir)
}

/// The one line of a failed command's stderr most likely to say why. Warnings
/// are skipped, since tools print those even when they succeed.
pub fn failure_line(stderr: &str) -> Option<String> {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !l.to_ascii_lowercase().contains(" warn "))
        .filter(|l| !l.to_ascii_lowercase().starts_with("npm warn"))
        .collect();
    let telling = [
        "error",
        "fatal",
        "not loaded",
        "denied",
        "not found",
        "no such",
    ];
    let pick = lines
        .iter()
        .find(|l| telling.iter().any(|t| l.to_ascii_lowercase().contains(t)))
        .or(lines.last())?;
    let clean: String = pick.replace('\t', " ");
    Some(if clean.chars().count() > 200 {
        format!("{}…", clean.chars().take(200).collect::<String>())
    } else {
        clean
    })
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
        measured: false,
        cache_bytes: None,
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
        let cache = measure_dir(&c.measure);
        let before = cache.as_deref().map(disk_usage);
        if !apply {
            out.status = Status::WouldRun;
            out.cache_bytes = before;
            return out;
        }
        // Run from the home folder. An app opened from Finder starts in `/`, and
        // pnpm, for one, exits with status 226 and no message when run there.
        match Command::new(&argv[0])
            .args(&argv[1..])
            .current_dir(home())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
        {
            Ok(o) if o.status.success() => out.status = Status::Ran,
            Ok(o) => {
                out.status = Status::Failed;
                // Some tools report their error on stdout, so look there too.
                out.reason = Some(
                    failure_line(&String::from_utf8_lossy(&o.stderr))
                        .or_else(|| failure_line(&String::from_utf8_lossy(&o.stdout)))
                        .unwrap_or_else(|| format!("exited with {}", o.status)),
                );
            }
            Err(e) => {
                out.status = Status::Failed;
                out.reason = Some(e.to_string());
            }
        }
        if let (Some(dir), Some(b)) = (cache, before) {
            out.bytes = b.saturating_sub(disk_usage(&dir));
            out.measured = true;
        }
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

/// The one-line form used when there are too many targets to list. It counts
/// what goes, not what matched: a target a filter kept is still in `targets`,
/// with the reason, and calling those removed would overstate the clean.
fn summary_line(verb: &str, o: &Outcome) -> String {
    let kept = o.targets.iter().filter(|t| t.kept.is_some()).count();
    let failed = o
        .targets
        .iter()
        .filter(|t| t.kept.is_none() && t.error.is_some())
        .count();
    let gone = o.targets.len() - kept - failed;
    let items = if gone == 1 { "item" } else { "items" };
    let mut line = format!("{verb} {gone} {items}, {}", human(o.bytes));
    if kept > 0 {
        line.push_str(&format!(", kept {kept}"));
    }
    if failed > 0 {
        line.push_str(&format!(", could not remove {failed}"));
    }
    line
}

pub fn print_text(c: &Cleaner, o: &Outcome) {
    println!("\n{}  ({})", c.name, c.summary);
    match o.status {
        Status::Skipped => println!("  skipped: {}", o.reason.as_deref().unwrap_or("")),
        Status::Nothing => println!("  nothing to clear"),
        Status::WouldRun => match o.cache_bytes {
            Some(b) => println!(
                "  would run: {}  (its cache is {} now; it may free less)",
                o.command.as_deref().unwrap_or(""),
                human(b)
            ),
            None => println!("  would run: {}", o.command.as_deref().unwrap_or("")),
        },
        Status::Ran => {
            let freed = if o.measured {
                format!("  (freed {})", human(o.bytes))
            } else {
                String::new()
            };
            println!("  ran: {}{freed}", o.command.as_deref().unwrap_or(""));
        }
        Status::Failed => {
            println!("  failed: {}", o.command.as_deref().unwrap_or(""));
            if let Some(r) = &o.reason {
                println!("          {r}");
            }
        }
        Status::WouldClear | Status::Cleared if o.targets.len() > 40 => {
            let verb = if o.status == Status::Cleared {
                "removed"
            } else {
                "would remove"
            };
            println!("  {}", summary_line(verb, o));
            // Only a cleaner that reads an application's database can say
            // this. Printed for every long list, it credited a glob with a
            // check nobody ran.
            if c.orphans.is_some() {
                println!("  every one of them checked against the application's own records");
            }
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
    fn picks_the_line_that_explains_a_failure() {
        let dyld = "dyld[96489]: Library not loaded: /opt/homebrew/opt/llhttp/lib/libllhttp.9.3.dylib\n  Referenced from: <x> /opt/homebrew/Cellar/node/25.8.1_1/bin/node\n  Reason: tried: a, b";
        assert!(failure_line(dyld).unwrap().contains("Library not loaded"));
        let npm = "npm warn using --force Recommended protections disabled.\nnpm error code EACCES";
        assert_eq!(failure_line(npm).unwrap(), "npm error code EACCES");
        assert_eq!(failure_line("  \n"), None);
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
    fn the_summary_counts_what_went_not_what_matched() {
        let target = |kept: Option<&str>, error: Option<&str>| Target {
            path: PathBuf::from("/x"),
            bytes: 1,
            kept: kept.map(Into::into),
            error: error.map(Into::into),
        };
        let o = Outcome {
            name: "t".into(),
            status: Status::Cleared,
            reason: None,
            command: None,
            targets: vec![
                target(None, None),
                target(None, None),
                target(Some("newest copy of a"), None),
                target(None, Some("permission denied")),
            ],
            bytes: 2_000_000,
            measured: false,
            cache_bytes: None,
        };
        assert_eq!(
            summary_line("removed", &o),
            "removed 2 items, 2.0 MB, kept 1, could not remove 1"
        );
    }

    #[test]
    fn name_before_version_cuts_at_the_version() {
        assert_eq!(
            name_before_version("anthropic.claude-code-2.1.276-darwin-arm64"),
            Some("anthropic.claude-code")
        );
        assert_eq!(
            name_before_version("ms-dotnettools.csharp-2.140.9-darwin-arm64"),
            Some("ms-dotnettools.csharp")
        );
        // Nothing to cut: these stay in a group of their own rather than
        // becoming rivals of everything else in the folder.
        assert_eq!(name_before_version("extensions.json"), None);
        assert_eq!(name_before_version(".obsolete"), None);
    }

    #[test]
    fn group_by_keeps_the_newest_of_each_thing_not_of_the_folder() {
        let root = std::env::temp_dir().join(format!("mac-headroom-gb-{}", now()));
        // One folder holding two versions of two extensions, the way VS Code
        // stores them, plus a file that has no version at all.
        for name in [
            "pub.alpha-1.0.0-darwin-arm64",
            "pub.alpha-2.0.0-darwin-arm64",
            "pub.beta-1.0.0-darwin-arm64",
            "pub.beta-2.0.0-darwin-arm64",
        ] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("extensions.json"), b"[]").unwrap();
        for old in [
            "pub.alpha-1.0.0-darwin-arm64",
            "pub.beta-1.0.0-darwin-arm64",
        ] {
            std::process::Command::new("touch")
                .args(["-t", "202001010000", root.join(old).to_str().unwrap()])
                .status()
                .unwrap();
        }

        let pattern = format!("{}/*", root.display());
        let mut c = super::paths("t", "", "because", None, &[&pattern]);
        c.keep_newest = Some(1);
        c.group_by = Some(GroupBy::NameBeforeVersion);
        let found = candidates(&c);
        let mut kept: Vec<String> = found
            .iter()
            .filter(|(_, k)| k.is_some())
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        kept.sort();
        // The newest of each extension, and the unversioned file, which is
        // never something's older copy.
        assert_eq!(
            kept,
            vec![
                "extensions.json",
                "pub.alpha-2.0.0-darwin-arm64",
                "pub.beta-2.0.0-darwin-arm64"
            ]
        );

        // Without the grouping the same cleaner would keep one folder and
        // delete the other three, which is the bug this field exists to stop.
        c.group_by = None;
        assert_eq!(
            candidates(&c).iter().filter(|(_, k)| k.is_some()).count(),
            1
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn group_by_needs_keep_newest() {
        let mut c = user("x", &["~/a/b"]);
        c.group_by = Some(GroupBy::NameBeforeVersion);
        assert!(c.validate().is_err());
        c.keep_newest = Some(1);
        assert!(c.validate().is_ok());
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
