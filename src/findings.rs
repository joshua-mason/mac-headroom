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
use crate::util::{disk_usage, home, is_running, now, state_dir};
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
    fixed(&mut out, h.join("Library/Application Support/MobileSync/Backup"), "iphone-backups", "iPhone and iPad backups",
        "Full backups of devices made on this Mac. They can be very large, and old ones are easy to forget.",
        "In Finder, select your device in the sidebar and choose Manage Backups to delete old ones.");

    let (node, venv, trees) = dependency_folders(&project_roots(&h));
    let trees_found: Vec<PathBuf> = trees.iter().map(|(p, _)| p.clone()).collect();
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
}
