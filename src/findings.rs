//! What a person with a full disk needs to know, gathered once and saved.
//!
//! Two lists. What can safely be freed now, sized per cleaner. And things
//! worth a look that only the person can judge: large items that are often a
//! surprise to someone who installed developer tools by following instructions,
//! each explained in plain words with how to deal with it safely.
//!
//! Gathering walks project folders, so it happens during a scan or the weekly
//! run and is saved. The report reads the saved copy and stays quick.

use crate::cleaners;
use crate::util::{disk_usage, home, human, is_running, now, state_dir};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Anything smaller is not worth someone's attention. Decimal MB, so the
/// cut-off is the number the report prints, not 4.8% above it.
const WORTH_A_LOOK: u64 = 200_000_000;

#[derive(Serialize, Deserialize)]
pub struct Findings {
    pub at: u64,
    pub reclaimable: Vec<cleaners::Estimate>,
    pub detections: Vec<Detection>,
    /// False when macOS privacy protection hid folders such as the Trash from
    /// the scan, which can leave a very large amount unexplained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_disk_access: Option<bool>,
    /// True when protected folders were deliberately left out to avoid privacy
    /// prompts, so the report can say exactly what it did not look at.
    #[serde(default)]
    pub skipped_protected: bool,
}

/// Something the tool can do about a detection, for a caller that offers a
/// button rather than a sentence. The work is an opt-in cleaner, so an action
/// goes through the same dry run, the same skip-while-running check and the
/// same audit log as everything else that deletes, and there is no second path
/// through which this tool removes anything.
#[derive(Serialize, Deserialize)]
pub struct Action {
    /// What a button should say.
    pub label: String,
    /// Run it with `clean --only <name> --yes`. Its size, whether an open app
    /// blocks it and how it describes itself are the cleaner's own answers,
    /// so a button and a dry run can never disagree.
    pub cleaner: cleaners::Estimate,
}

/// One thing inside a detection that can be judged on its own, for the
/// detections where "some of these are fine to remove" is the honest answer.
#[derive(Serialize, Deserialize)]
pub struct Item {
    pub path: String,
    pub bytes: u64,
    /// True only when removing it would lose nothing, and that was checked
    /// rather than assumed. Anything that could not be checked is not safe.
    pub safe: bool,
    /// What was found, in words.
    pub note: String,
    /// The command that removes it properly. Only given when `safe`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
}

#[derive(Serialize, Deserialize)]
pub struct Detection {
    pub id: String,
    pub title: String,
    pub bytes: u64,
    /// What this is, for someone who may not know they have it.
    pub what: String,
    /// How to deal with it safely. Advice, never an action the tool takes.
    pub how: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The largest individual items, where there are many.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub largest: Vec<(String, u64)>,
    /// What can be done about this without leaving the app. `how` still says
    /// what to do in words, because most of these have no action and never
    /// will: the ones that are someone's own files, or that need a judgement
    /// no tool should make for them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    /// The individual things found, where each can be judged separately.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Item>,
}

/// Detections something can be done about, and the cleaner that does it.
/// Deliberately short. A detection earns an action only when the work is
/// rebuildable or re-downloadable, needs no sudo, and frees a figure the tool
/// can stand behind afterwards. Docker is the instructive exclusion: pruning
/// its build cache frees space inside a disk image that does not shrink, so a
/// button would report a number the disk would not agree with.
const ACTIONS: [(&str, &str, &str); 3] = [
    ("claude-vm", "claude-vm-bundles", "Delete the sandbox image"),
    (
        "device-support",
        "xcode-device-support",
        "Delete the device support files",
    ),
    (
        "simulators",
        "simulators-unavailable",
        "Delete simulators that cannot boot",
    ),
];

fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', "'\\''"))
}

/// Each working copy under a project's `.claude/worktrees`, with whether
/// removing it would lose anything. A working copy is safe only when git says
/// so: nothing uncommitted, and no commit that exists nowhere but here. A
/// folder git cannot answer for is reported as unchecked, never as safe,
/// because an empty answer from a failed command looks exactly like a clean one.
fn worktree_items(dirs: &[PathBuf]) -> Vec<Item> {
    let mut out = Vec::new();
    for dir in dirs {
        // <repo>/.claude/worktrees
        let Some(repo) = dir.parent().and_then(Path::parent) else {
            continue;
        };
        let Ok(children) = fs::read_dir(dir) else {
            continue;
        };
        for wt in children.filter_map(Result::ok).map(|e| e.path()) {
            if !wt.is_dir() {
                continue;
            }
            let bytes = disk_usage(&wt);
            let p = wt.to_string_lossy().into_owned();
            let git = |args: &[&str]| {
                let mut a = vec!["-C", p.as_str()];
                a.extend_from_slice(args);
                crate::util::stdout_of("git", &a)
            };
            // The folder has to be the root of its own working copy. Asking
            // only "is this inside a work tree" is answered yes by any plain
            // folder in the project, on behalf of the project around it.
            let top = git(&["rev-parse", "--show-toplevel"]);
            let checked = !top.trim().is_empty()
                && fs::canonicalize(top.trim()).ok() == fs::canonicalize(&wt).ok();
            let dirty = git(&["status", "--porcelain"]).lines().count();
            let unpushed = git(&["log", "--oneline", "HEAD", "--not", "--remotes"])
                .lines()
                .count();
            let touched = fs::metadata(&wt)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| crate::util::ago(d.as_secs()))
                .unwrap_or_else(|| "at an unknown time".into());
            let safe = checked && dirty == 0 && unpushed == 0;
            let note = if !checked {
                "git could not check this one, so it is left to you".to_string()
            } else if safe {
                format!("nothing uncommitted and nothing unpushed, last touched {touched}")
            } else {
                let mut parts = Vec::new();
                if dirty > 0 {
                    parts.push(format!(
                        "{dirty} uncommitted file{}",
                        if dirty == 1 { "" } else { "s" }
                    ));
                }
                if unpushed > 0 {
                    parts.push(format!(
                        "{unpushed} commit{} on no remote",
                        if unpushed == 1 { "" } else { "s" }
                    ));
                }
                format!("{}, last touched {touched}", parts.join(" and "))
            };
            out.push(Item {
                path: p.clone(),
                bytes,
                safe,
                note,
                command: if safe {
                    format!(
                        "git -C {} worktree remove {}",
                        shell_quote(repo),
                        shell_quote(&wt)
                    )
                } else {
                    String::new()
                },
            });
        }
    }
    out.sort_by_key(|i| std::cmp::Reverse(i.bytes));
    out
}

/// After a clean, re-measures the detections whose action just ran. Without
/// this the saved findings go on showing the size from before, so an action
/// that worked looks like one that did nothing. Only the detections touched
/// are measured again: the full set includes a walk of every project folder,
/// which is a scan's job and too slow to hang off a button.
pub fn refresh_after(ran: &[String], saved: &mut Findings) {
    let all = crate::config::all_cleaners().unwrap_or_default();
    for d in saved.detections.iter_mut() {
        if !d.actions.iter().any(|a| ran.contains(&a.cleaner.name)) {
            continue;
        }
        if let Some(path) = &d.path {
            d.bytes = disk_usage(Path::new(path));
        }
        for a in d.actions.iter_mut() {
            if let Some(c) = cleaners::find(&all, &a.cleaner.name) {
                a.cleaner = cleaners::estimate(c);
            }
        }
    }
    // The same bar a scan applies: what is no longer large is no longer listed.
    saved.detections.retain(|d| {
        d.bytes >= WORTH_A_LOOK || !d.actions.iter().any(|a| ran.contains(&a.cleaner.name))
    });
    saved.detections.sort_by_key(|d| std::cmp::Reverse(d.bytes));
}

/// Attaches each detection's action, measured the same way the clear list is.
fn attach_actions(out: &mut [Detection]) {
    let all = crate::config::all_cleaners().unwrap_or_default();
    for d in out.iter_mut() {
        for (detection, cleaner, label) in ACTIONS {
            if d.id != detection {
                continue;
            }
            // A cleaner switched off in the config is switched off here too.
            // One list of what this Mac is willing to delete, not two.
            if let Some(c) = cleaners::find(&all, cleaner).filter(|c| !c.disabled) {
                d.actions.push(Action {
                    label: label.into(),
                    cleaner: cleaners::estimate(c),
                });
            }
        }
    }
}

fn path_file() -> PathBuf {
    state_dir().join("findings.json")
}

pub fn save(f: &Findings) {
    if let Ok(json) = serde_json::to_string(f) {
        let _ = fs::write(path_file(), json);
    }
}

pub fn load() -> Option<Findings> {
    serde_json::from_str(&fs::read_to_string(path_file()).ok()?).ok()
}

/// The saved findings, with the one part of them that goes stale in seconds
/// checked again: which apps are open. Sizes are a scan's measurement and stay
/// as saved, but "quit Chrome first" is a fact about this moment. Read from the
/// file, it told someone who had already quit the app to quit it, and anything
/// that acts on the list left that cleaner out of a clear it could have run.
/// Costs a process lookup per cleaner that names an app, and writes nothing.
pub fn load_current() -> Option<Findings> {
    let mut f = load()?;
    let all = crate::config::all_cleaners().unwrap_or_default();
    let blocker = |name: &str| {
        cleaners::find(&all, name)
            .and_then(|c| c.skip_if_running.clone())
            .filter(|app| is_running(app))
    };
    for e in f.reclaimable.iter_mut() {
        e.blocked_by = blocker(&e.name);
    }
    for d in f.detections.iter_mut() {
        for a in d.actions.iter_mut() {
            a.cleaner.blocked_by = blocker(&a.cleaner.name);
        }
    }
    Some(f)
}

/// One line of `docker system df`.
#[derive(Debug, PartialEq)]
struct DockerUsage {
    kind: String,
    count: u64,
    reclaimable: u64,
}

/// Docker prints sizes its own way, "4.102GB" or "512kB", in decimal units.
fn parse_docker_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (number, unit) = text.split_at(split);
    let scale = match unit.trim() {
        "B" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        _ => return None,
    };
    Some((number.parse::<f64>().ok()? * scale) as u64)
}

/// `docker system df --format '{{json .}}'`: one JSON object a line, every
/// value a string, and Reclaimable either "0B" or "3.65GB (89%)".
fn parse_docker_df(text: &str) -> Vec<DockerUsage> {
    text.lines()
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            let reclaimable = v.get("Reclaimable")?.as_str()?;
            Some(DockerUsage {
                kind: v.get("Type")?.as_str()?.to_string(),
                count: v.get("TotalCount")?.as_str()?.parse().ok()?,
                reclaimable: parse_docker_size(reclaimable.split(' ').next()?)?,
            })
        })
        .collect()
}

/// What to say about Docker's disk once Docker has been asked. The figures
/// are Docker's own and are given as such. Volumes are counted out: they are
/// where databases live, and nothing here should nudge anyone to prune them.
fn docker_advice(rows: &[DockerUsage]) -> Option<String> {
    let of = |kind: &str| rows.iter().find(|r| r.kind == kind);
    let images = of("Images").map_or(0, |r| r.reclaimable);
    let cache = of("Build Cache").map_or(0, |r| r.reclaimable);
    let containers = of("Containers").map_or(0, |r| r.reclaimable);
    if rows.is_empty() {
        return None;
    }
    let volumes = match of("Local Volumes") {
        Some(v) if v.count > 0 => format!(
            " It also holds {} volume{}, which is where databases usually live. Leave those unless you know each one.",
            v.count,
            if v.count == 1 { "" } else { "s" }
        ),
        _ => String::new(),
    };
    let total = images + cache + containers;
    if total < 100_000_000 {
        return Some(format!(
            "Docker reports almost nothing inside it as unused, so pruning would free little.{volumes}"
        ));
    }
    let mut parts = Vec::new();
    if images > 0 {
        parts.push(format!(
            "{} of images no container uses (`docker image prune -a`)",
            human(images)
        ));
    }
    if cache > 0 {
        parts.push(format!(
            "{} of build cache (`docker builder prune`)",
            human(cache)
        ));
    }
    if containers > 0 {
        parts.push(format!(
            "{} in stopped containers (`docker container prune`)",
            human(containers)
        ));
    }
    Some(format!(
        "Docker reports {} inside it as unused: {}. That is Docker's figure, not a measurement of this disk, and how much of it the file hands back is up to Docker Desktop.{volumes}",
        human(total),
        parts.join(", ")
    ))
}

/// Ask Docker what it considers unused, and say that instead of the generic
/// advice. Images pulled by a tool on someone's behalf, with no container
/// using them, were most of a disk image that doubled in a day here.
fn explain_docker(out: &mut [Detection]) {
    let Some(d) = out.iter_mut().find(|d| d.id == "docker") else {
        return;
    };
    let asked =
        crate::util::stdout_within("docker", &["system", "df", "--format", "{{json .}}"], 10);
    match asked.and_then(|text| docker_advice(&parse_docker_df(&text))) {
        Some(advice) => d.how = advice,
        None => d.how.push_str(
            " Docker is not running, so it could not be asked how much of this is unused. Start Docker Desktop and scan again to see.",
        ),
    }
}

fn fixed(out: &mut Vec<Detection>, path: PathBuf, id: &str, title: &str, what: &str, how: &str) {
    if crate::util::skip_protected(&path) || !path.exists() {
        return;
    }
    let bytes = disk_usage(&path);
    if bytes >= WORTH_A_LOOK {
        out.push(Detection {
            id: id.into(),
            title: title.into(),
            bytes,
            what: what.into(),
            how: how.into(),
            path: Some(path.display().to_string()),
            largest: vec![],
            actions: vec![],
            items: vec![],
        });
    }
}

/// Folders people keep projects in. Only these are searched for dependency
/// folders, so the walk stays fast and never wanders through system data.
fn project_roots(h: &Path) -> Vec<PathBuf> {
    let names = [
        "git",
        "code",
        "Code",
        "Developer",
        "Projects",
        "projects",
        "src",
        "dev",
        "repos",
        "Documents",
        "Desktop",
    ];
    let mut roots: Vec<PathBuf> = names
        .iter()
        .map(|n| h.join(n))
        .filter(|p| !crate::util::skip_protected(p) && p.is_dir())
        .collect();
    // Drop any root already inside another, so nothing is counted twice.
    roots.sort();
    let mut kept: Vec<PathBuf> = Vec::new();
    for r in roots {
        if !kept.iter().any(|k| r.starts_with(k)) {
            kept.push(r);
        }
    }
    kept
}

/// Folders found, each with its size on disk.
type Sized = Vec<(PathBuf, u64)>;

/// How many dependency folders to judge one by one. Each costs a few file
/// checks and a git call, and a page listing two hundred helps nobody.
const ITEMS_LISTED: usize = 15;

#[derive(Clone, Copy)]
enum Rebuilt {
    NodeModules,
    Venv,
}

/// Whether a file with one of these names sits in `dir` or a folder above it,
/// stopping at the top of the repository. A package in a monorepo keeps its
/// node_modules beside its own package.json and its lock file at the root.
fn found_upwards(dir: &Path, names: &[&str]) -> bool {
    for folder in dir.ancestors().take(5) {
        if names.iter().any(|n| folder.join(n).is_file()) {
            return true;
        }
        if folder.join(".git").exists() {
            break;
        }
    }
    false
}

/// Whether deleting a dependency folder loses nothing, and why. It loses
/// nothing only when a lock file can rebuild it exactly. A manifest without
/// one brings back whatever versions are current that day, which is usually
/// fine and is not the same thing, so that is left to the person.
fn rebuild_verdict(folder: &Path, kind: Rebuilt) -> (bool, String) {
    let Some(project) = folder.parent() else {
        return (false, "nothing says how to rebuild it".into());
    };
    let (manifests, locks): (&[&str], &[&str]) = match kind {
        Rebuilt::NodeModules => (
            &["package.json"],
            &[
                "package-lock.json",
                "pnpm-lock.yaml",
                "yarn.lock",
                "bun.lock",
                "bun.lockb",
                "npm-shrinkwrap.json",
            ],
        ),
        Rebuilt::Venv => (
            &[
                "pyproject.toml",
                "requirements.txt",
                "Pipfile",
                "setup.py",
                "environment.yml",
            ],
            &["uv.lock", "poetry.lock", "Pipfile.lock", "pdm.lock"],
        ),
    };
    if !manifests.iter().any(|m| project.join(m).is_file()) {
        let expected = match kind {
            Rebuilt::NodeModules => "no package.json beside it",
            Rebuilt::Venv => "no project file beside it",
        };
        return (
            false,
            format!("{expected}, so nothing says how to rebuild it"),
        );
    }
    if found_upwards(project, locks) {
        (
            true,
            "a lock file is kept with it, so reinstalling rebuilds it exactly".to_string(),
        )
    } else {
        (
            false,
            "no lock file, so reinstalling may bring back different versions".to_string(),
        )
    }
}

/// When the project around a folder was last worked on: its last commit, or
/// failing that the last change to the folder holding it.
fn last_worked_on(project: &Path) -> Option<u64> {
    let p = project.to_string_lossy();
    crate::util::stdout_of("git", &["-C", &p, "log", "-1", "--format=%ct"])
        .trim()
        .parse()
        .ok()
        .or_else(|| {
            fs::metadata(project)
                .ok()?
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .map(|d| d.as_secs())
        })
}

/// The largest dependency folders, each with whether it can be rebuilt
/// exactly and how long the project has sat untouched. The tool removes none
/// of them: which projects someone is finished with is theirs to say.
fn rebuildable_items(found: &[(PathBuf, u64)], kind: Rebuilt) -> Vec<Item> {
    let mut found: Vec<&(PathBuf, u64)> = found.iter().collect();
    found.sort_by_key(|(_, b)| std::cmp::Reverse(*b));
    found
        .into_iter()
        .take(ITEMS_LISTED)
        .map(|(path, bytes)| {
            let (safe, why) = rebuild_verdict(path, kind);
            let worked = path
                .parent()
                .and_then(last_worked_on)
                .map(|at| format!(", project last worked on {}", crate::util::ago(at)))
                .unwrap_or_default();
            Item {
                path: path.display().to_string(),
                bytes: *bytes,
                safe,
                note: format!("{why}{worked}"),
                command: if safe {
                    format!("rm -rf {}", shell_quote(path))
                } else {
                    String::new()
                },
            }
        })
        .collect()
}

/// Dependency folders a project re-creates on install: node_modules, and
/// Python virtual environments (recognised by their pyvenv.cfg).
fn dependency_folders(roots: &[PathBuf]) -> (Sized, Sized, Sized) {
    let (mut node, mut venv, mut trees) = (Vec::new(), Vec::new(), Vec::new());
    for root in roots {
        let mut it = walkdir::WalkDir::new(root)
            .max_depth(6)
            .follow_links(false)
            .into_iter();
        while let Some(entry) = it.next() {
            let Ok(e) = entry else { continue };
            if !e.file_type().is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy();
            if crate::util::skip_protected(e.path()) {
                it.skip_current_dir();
            } else if name == "node_modules" {
                node.push((e.path().to_path_buf(), disk_usage(e.path())));
                it.skip_current_dir();
            } else if name == "worktrees"
                && e.path().parent().is_some_and(|p| p.ends_with(".claude"))
            {
                trees.push((e.path().to_path_buf(), disk_usage(e.path())));
                it.skip_current_dir();
            } else if name == ".git" || name == "Library" || name == ".Trash" {
                it.skip_current_dir();
            } else if e.path().join("pyvenv.cfg").is_file() {
                venv.push((e.path().to_path_buf(), disk_usage(e.path())));
                it.skip_current_dir();
            }
        }
    }
    (node, venv, trees)
}

fn grouped(out: &mut Vec<Detection>, items: Sized, id: &str, title: &str, what: &str, how: &str) {
    let bytes: u64 = items.iter().map(|(_, b)| b).sum();
    if bytes < WORTH_A_LOOK {
        return;
    }
    let mut items = items;
    items.sort_by_key(|(_, b)| std::cmp::Reverse(*b));
    out.push(Detection {
        id: id.into(),
        title: format!("{title} ({} folders)", items.len()),
        bytes,
        what: what.into(),
        how: how.into(),
        path: None,
        largest: items
            .into_iter()
            .take(5)
            .map(|(p, b)| (p.display().to_string(), b))
            .collect(),
        actions: vec![],
        items: vec![],
    });
}

pub fn detections() -> Vec<Detection> {
    let h = home();
    let mut out = Vec::new();

    fixed(
        &mut out,
        h.join(".Trash"),
        "trash",
        "Your Trash",
        "Files you deleted still use space until the Trash is emptied.",
        "Empty the Trash from Finder once you are sure nothing in it is needed.",
    );
    fixed(&mut out, h.join("Downloads"), "downloads", "Your Downloads folder",
        "Everything you have downloaded. Installer files (.dmg, .pkg, .zip) are rarely needed again once the app is installed.",
        "In Finder, open Downloads, choose View > as List, and sort by Size to find the largest files.");
    fixed(&mut out, h.join("Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw"), "docker", "Docker's virtual disk",
        "Docker Desktop keeps images, containers and their data inside one large file, and it does not shrink on its own.",
        "Run `docker system prune -a` to remove images and stopped containers you are not using. It keeps volumes, which is where databases usually live.");
    explain_docker(&mut out);
    fixed(&mut out, h.join("Library/Developer/CoreSimulator/Devices"), "simulators", "iPhone and iPad simulators",
        "Simulated devices created by Xcode, each with its own apps and data.",
        "Run `xcrun simctl delete unavailable` to remove simulators for iOS versions you no longer have installed.");
    fixed(&mut out, h.join("Library/Developer/Xcode/iOS DeviceSupport"), "device-support", "Xcode device support files",
        "Debugging files Xcode copies from each iPhone version you have connected.",
        "Folders for iOS versions your devices no longer run can be deleted; Xcode copies them again if needed.");
    fixed(&mut out, h.join("Library/Developer/Xcode/Archives"), "archives", "Xcode archives",
        "Copies of app builds you exported or submitted.",
        "Keep the ones for app versions people still use, since they are needed to read crash reports. Older ones can go.");
    fixed(&mut out, h.join(".cache/huggingface"), "huggingface", "AI models from Hugging Face",
        "Model files downloaded by Python AI tools. A single model can be several gigabytes.",
        "Each model is a folder under `~/.cache/huggingface/hub` named after it. Delete the folders for models you no longer use; a tool that needs one downloads it again.");
    fixed(
        &mut out,
        h.join(".ollama/models"),
        "ollama",
        "AI models from Ollama",
        "Language models you have pulled to run locally. Each is typically several gigabytes.",
        "Run `ollama list` to see them and `ollama rm <name>` for any you no longer use.",
    );
    fixed(
        &mut out,
        h.join(".lmstudio/models"),
        "lmstudio",
        "AI models from LM Studio",
        "Language models downloaded to run locally.",
        "Remove models you no longer use from LM Studio's My Models page.",
    );
    fixed(&mut out, h.join("Library/Application Support/Claude/vm_bundles"), "claude-vm", "Claude Desktop's sandbox disk image",
        "A virtual machine image Claude Desktop keeps so it can run code away from your own files. It is made once and grows as it is used.",
        "If you do not use Claude Desktop, or never let it run code, quit it and delete the folder. It is built again the next time the sandbox is needed.");
    fixed(&mut out, PathBuf::from("/System/Library/AssetsV2/com_apple_MobileAsset_iOSSimulatorRuntime"), "simulator-runtimes", "Installed iOS simulator runtimes",
        "The iOS versions Xcode can simulate. Each one is a complete copy of iOS and is several gigabytes, whether or not you still build for it.",
        "Run `xcrun simctl runtime list` to see them, then `xcrun simctl runtime delete <build>` for any you no longer build against. Xcode offers to download one again when you need it.");
    fixed(&mut out, PathBuf::from("/Library/Developer/CoreSimulator/Caches"), "simulator-caches", "Simulator startup caches",
        "Caches the simulator builds for each macOS version it has run under. Folders for macOS versions this Mac no longer runs are dead weight.",
        "They are rebuilt on demand, so `sudo rm -rf /Library/Developer/CoreSimulator/Caches/*` costs nothing but a slower first simulator launch.");
    // GarageBand ships on every Mac, and opening it once downloads a sound
    // library most people never think about again. It is spread over three
    // folders, so it is measured as one thing rather than three small ones.
    let music: u64 = [
        "/Library/Application Support/GarageBand",
        "/Library/Application Support/Logic",
        "/Library/Audio/Apple Loops",
    ]
    .iter()
    .map(|p| disk_usage(Path::new(p)))
    .sum();
    if music >= WORTH_A_LOOK {
        out.push(Detection {
            id: "music-creation".into(),
            title: "GarageBand's sound library".into(),
            bytes: music,
            what: "Instruments and loops GarageBand downloaded, shared with Logic. They stay even if you only opened GarageBand once.".into(),
            how: "Open System Settings > General > Storage, choose Music Creation, and remove the sound library there, which clears it properly. GarageBand offers to download it again from its Sound Library menu.".into(),
            path: None,
            largest: vec![],
            actions: vec![],
            items: vec![],
        });
    }
    fixed(&mut out, h.join("Library/Application Support/MobileSync/Backup"), "iphone-backups", "iPhone and iPad backups",
        "Full backups of devices made on this Mac. They can be very large, and old ones are easy to forget.",
        "In Finder, select your device in the sidebar and choose Manage Backups to delete old ones.");

    let (node, venv, trees) = dependency_folders(&project_roots(&h));
    let trees_found: Vec<PathBuf> = trees.iter().map(|(p, _)| p.clone()).collect();
    let (node_found, venv_found) = (node.clone(), venv.clone());
    grouped(&mut out, node, "node-modules", "JavaScript project dependencies",
        "Each JavaScript project keeps a node_modules folder of the libraries it uses. They add up quickly across many projects.",
        "Delete node_modules in projects you are not working on. Running `npm install` in a project brings it back.");
    grouped(&mut out, venv, "python-venvs", "Python virtual environments",
        "Each Python project can have its own environment holding the libraries it uses.",
        "Delete environments for projects you are not working on. They can be recreated from the project's requirements.");
    grouped(&mut out, trees, "agent-worktrees", "Working copies left by coding agents",
        "A coding agent that works on several things at once gives each one its own checkout of the project, under the project's .claude folder. They are rarely cleared up afterwards.",
        "These are git working copies, so check for uncommitted work first. In the project, `git worktree list` shows them and `git worktree remove <path>` removes one properly. Deleting the folder by hand leaves git holding a reference to it.");

    if let Some(d) = out.iter_mut().find(|d| d.id == "agent-worktrees") {
        d.items = worktree_items(&trees_found);
    }
    // Judged only once the group is known to be worth listing at all.
    for (id, found, kind) in [
        ("node-modules", node_found, Rebuilt::NodeModules),
        ("python-venvs", venv_found, Rebuilt::Venv),
    ] {
        if let Some(d) = out.iter_mut().find(|d| d.id == id) {
            d.items = rebuildable_items(&found, kind);
            if found.len() > ITEMS_LISTED {
                d.what
                    .push_str(&format!(" The {ITEMS_LISTED} largest are listed."));
            }
        }
    }

    if !crate::util::skipping_protected() && !is_running("WhatsApp") {
        if let Ok(orphans) = crate::orphans::find(&crate::orphans::WHATSAPP) {
            let bytes: u64 = orphans.iter().map(|p| disk_usage(p)).sum();
            if bytes >= WORTH_A_LOOK {
                out.push(Detection {
                    id: "whatsapp-orphans".into(),
                    title: format!("WhatsApp files it has forgotten ({} files)", orphans.len()),
                    bytes,
                    what: "Photos and videos WhatsApp downloaded and then lost track of, usually after you logged in again. WhatsApp's own storage settings cannot see or remove them.".into(),
                    how: "Check that your chats look right on your phone, quit WhatsApp, then run `mac-headroom clean --only whatsapp-orphans --yes`. Anything still on your phone downloads again if you scroll back.".into(),
                    path: None,
                    largest: vec![],
                    actions: vec![],
            items: vec![],
                });
            }
        }
    }

    out.sort_by_key(|d| std::cmp::Reverse(d.bytes));
    attach_actions(&mut out);
    out
}

pub fn gather() -> Findings {
    let reclaimable = crate::config::all_cleaners()
        .unwrap_or_default()
        .iter()
        .filter(|c| !c.disabled && !c.opt_in)
        .map(cleaners::estimate)
        .collect();
    Findings {
        at: now(),
        reclaimable,
        detections: detections(),
        full_disk_access: crate::util::full_disk_access(),
        skipped_protected: crate::util::skipping_protected(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCKER_DF: &str = r#"{"Active":"1","Reclaimable":"3.65GB (89%)","Size":"4.102GB","TotalCount":"3","Type":"Images"}
{"Active":"1","Reclaimable":"0B","Size":"12.3kB","TotalCount":"2","Type":"Containers"}
{"Active":"2","Reclaimable":"1.2GB","Size":"9.8GB","TotalCount":"34","Type":"Local Volumes"}
{"Active":"0","Reclaimable":"512.5MB","Size":"512.5MB","TotalCount":"40","Type":"Build Cache"}
"#;

    #[test]
    fn docker_sizes_are_decimal_and_carry_their_unit() {
        assert_eq!(parse_docker_size("0B"), Some(0));
        assert_eq!(parse_docker_size("12.3kB"), Some(12_300));
        assert_eq!(parse_docker_size("4.102GB"), Some(4_102_000_000));
        assert_eq!(parse_docker_size("lots"), None);
        assert_eq!(parse_docker_size("5"), None);
    }

    #[test]
    fn docker_advice_names_what_is_unused_and_leaves_volumes_out_of_it() {
        let rows = parse_docker_df(DOCKER_DF);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].reclaimable, 3_650_000_000);
        let advice = docker_advice(&rows).unwrap();
        // Images and build cache, and not the 1.2 GB Docker calls reclaimable
        // in volumes: those are someone's databases.
        assert!(
            advice.starts_with("Docker reports 4.2 GB inside it as unused"),
            "{advice}"
        );
        assert!(
            advice.contains("docker image prune -a") && advice.contains("docker builder prune")
        );
        assert!(!advice.contains("container prune"));
        assert!(advice.contains("34 volumes"));
        assert!(!advice.contains("volume prune"));
    }

    #[test]
    fn docker_advice_says_so_when_there_is_nothing_to_prune() {
        let empty = parse_docker_df(
            r#"{"Active":"0","Reclaimable":"0B","Size":"0B","TotalCount":"0","Type":"Images"}"#,
        );
        let advice = docker_advice(&empty).unwrap();
        assert!(
            advice.starts_with("Docker reports almost nothing"),
            "{advice}"
        );
        assert!(!advice.contains("volume"));
        // Docker did not answer at all: nothing to say, keep the general advice.
        assert_eq!(
            docker_advice(&parse_docker_df("Cannot connect to the Docker daemon")),
            None
        );
    }

    /// A typo in ACTIONS would not fail to compile: it would silently leave a
    /// detection with no button, which is the failure nobody notices.
    #[test]
    fn every_action_names_an_opt_in_built_in() {
        let all = cleaners::builtins();
        for (detection, cleaner, label) in ACTIONS {
            let c = cleaners::find(&all, cleaner)
                .unwrap_or_else(|| panic!("{detection}: there is no cleaner called {cleaner}"));
            assert!(
                c.opt_in,
                "{cleaner} has to be opt in, or a plain clean would run it without being asked"
            );
            assert!(!label.is_empty(), "{cleaner}: a button needs words on it");
        }
    }

    fn detection(id: &str, path: &Path, cleaner: &str) -> Detection {
        Detection {
            id: id.into(),
            title: id.into(),
            bytes: 1_000_000_000,
            what: String::new(),
            how: String::new(),
            path: Some(path.display().to_string()),
            largest: vec![],
            actions: vec![Action {
                label: "Delete".into(),
                cleaner: cleaners::Estimate {
                    name: cleaner.into(),
                    title: String::new(),
                    plain: String::new(),
                    bytes: Some(1_000_000_000),
                    upper_bound: false,
                    blocked_by: None,
                    note: None,
                    opt_in: true,
                },
            }],
            items: vec![],
        }
    }

    #[test]
    fn an_action_that_ran_stops_being_listed_at_its_old_size() {
        let root = std::env::temp_dir().join(format!("mac-headroom-ra-{}", now()));
        fs::create_dir_all(&root).unwrap();
        let mut saved = Findings {
            at: 0,
            reclaimable: vec![],
            detections: vec![
                detection("claude-vm", &root, "claude-vm-bundles"),
                detection("device-support", &root, "xcode-device-support"),
            ],
            full_disk_access: None,
            skipped_protected: false,
        };
        // The folder is empty now, as it would be after the action ran.
        refresh_after(&["claude-vm-bundles".to_string()], &mut saved);
        let ids: Vec<&str> = saved.detections.iter().map(|d| d.id.as_str()).collect();
        // The one that ran is gone. The other was not measured again, and
        // keeps the figure the last scan gave it.
        assert_eq!(ids, vec!["device-support"]);
        assert_eq!(saved.detections[0].bytes, 1_000_000_000);
        let _ = fs::remove_dir_all(&root);
    }

    fn git(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(args)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?} failed in {}", dir.display());
    }

    #[test]
    fn a_worktree_is_safe_only_when_git_says_nothing_would_be_lost() {
        let root = std::env::temp_dir().join(format!("mac-headroom-wt-{}", now()));
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        git(&root, &["init", "-q", "--bare", "remote.git"]);
        git(&repo, &["init", "-q"]);
        fs::write(repo.join("f"), "x").unwrap();
        git(&repo, &["add", "f"]);
        git(&repo, &["commit", "-q", "-m", "one"]);
        git(&repo, &["remote", "add", "origin", "../remote.git"]);
        git(&repo, &["push", "-q", "origin", "HEAD:refs/heads/main"]);
        git(&repo, &["fetch", "-q", "origin"]);
        let trees = repo.join(".claude/worktrees");
        fs::create_dir_all(&trees).unwrap();
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "clean",
                ".claude/worktrees/clean",
            ],
        );
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "dirty",
                ".claude/worktrees/dirty",
            ],
        );
        fs::write(trees.join("dirty/new"), "y").unwrap();
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "ahead",
                ".claude/worktrees/ahead",
            ],
        );
        fs::write(trees.join("ahead/g"), "z").unwrap();
        git(&trees.join("ahead"), &["add", "g"]);
        git(&trees.join("ahead"), &["commit", "-q", "-m", "only here"]);
        // Not a working copy at all. A failed git call returns nothing, which
        // must never be read as "nothing to lose".
        fs::create_dir_all(trees.join("notgit")).unwrap();

        let items = worktree_items(std::slice::from_ref(&trees));
        let by = |n: &str| items.iter().find(|i| i.path.ends_with(n)).unwrap();
        assert!(by("clean").safe, "{}", by("clean").note);
        assert!(by("clean").command.contains("worktree remove"));
        assert!(!by("dirty").safe && by("dirty").note.contains("1 uncommitted file"));
        assert!(!by("ahead").safe && by("ahead").note.contains("1 commit on no remote"));
        assert!(!by("notgit").safe && by("notgit").command.is_empty());
        assert!(by("dirty").command.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    /// Every action is a deletion, so it answers to the same rule as any
    /// cleaner: it says why it is safe.
    #[test]
    fn every_action_says_why_it_is_safe() {
        let all = cleaners::builtins();
        for (_, cleaner, _) in ACTIONS {
            let c = cleaners::find(&all, cleaner).unwrap();
            c.validate().unwrap_or_else(|e| panic!("{e}"));
            assert!(c.why_safe.len() > 40, "{cleaner}: why_safe is too thin");
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mac-headroom-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Safe means checked: only a lock file makes a rebuilt folder the same
    /// folder. A manifest alone does not, and nothing at all certainly not.
    #[test]
    fn a_dependency_folder_is_safe_only_with_a_lock_file() {
        let root = scratch("verdict");
        for p in ["locked", "loose", "bare"] {
            fs::create_dir_all(root.join(p).join("node_modules")).unwrap();
        }
        fs::write(root.join("locked/package.json"), "{}").unwrap();
        fs::write(root.join("locked/pnpm-lock.yaml"), "").unwrap();
        fs::write(root.join("loose/package.json"), "{}").unwrap();

        let verdict =
            |p: &str| rebuild_verdict(&root.join(p).join("node_modules"), Rebuilt::NodeModules);
        assert!(verdict("locked").0);
        let (safe, why) = verdict("loose");
        assert!(!safe && why.starts_with("no lock file"), "{why}");
        let (safe, why) = verdict("bare");
        assert!(!safe && why.starts_with("no package.json"), "{why}");
        fs::remove_dir_all(&root).unwrap();
    }

    /// A workspace package has its lock file at the top of the repository,
    /// and the search for one must not wander out of the repository.
    #[test]
    fn a_lock_file_is_found_at_the_top_of_the_repository_and_no_higher() {
        let root = scratch("upwards");
        let pkg = root.join("repo/packages/web");
        fs::create_dir_all(pkg.join("node_modules")).unwrap();
        fs::create_dir_all(root.join("repo/.git")).unwrap();
        fs::write(pkg.join("package.json"), "{}").unwrap();
        fs::write(root.join("yarn.lock"), "").unwrap();
        assert!(!rebuild_verdict(&pkg.join("node_modules"), Rebuilt::NodeModules).0);
        fs::write(root.join("repo/yarn.lock"), "").unwrap();
        assert!(rebuild_verdict(&pkg.join("node_modules"), Rebuilt::NodeModules).0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn only_the_largest_folders_are_listed_and_only_safe_ones_get_a_command() {
        let root = scratch("items");
        let mut found = Vec::new();
        for i in 0..(ITEMS_LISTED as u64 + 5) {
            let project = root.join(format!("p{i}"));
            fs::create_dir_all(project.join(".venv")).unwrap();
            fs::write(project.join("pyproject.toml"), "").unwrap();
            if i % 2 == 0 {
                fs::write(project.join("uv.lock"), "").unwrap();
            }
            found.push((project.join(".venv"), 1000 + i));
        }
        let items = rebuildable_items(&found, Rebuilt::Venv);
        assert_eq!(items.len(), ITEMS_LISTED);
        assert_eq!(items[0].bytes, 1000 + ITEMS_LISTED as u64 + 4);
        for item in &items {
            assert_eq!(item.safe, !item.command.is_empty());
            assert_eq!(item.safe, item.command.starts_with("rm -rf '"));
        }
        assert!(items.iter().any(|i| i.safe) && items.iter().any(|i| !i.safe));
        fs::remove_dir_all(&root).unwrap();
    }
}
