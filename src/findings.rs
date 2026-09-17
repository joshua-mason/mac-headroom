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

/// Anything smaller is not worth someone's attention.
const WORTH_A_LOOK: u64 = 200 << 20;

#[derive(Serialize, Deserialize)]
pub struct Findings {
    pub at: u64,
    pub reclaimable: Vec<cleaners::Estimate>,
    pub detections: Vec<Detection>,
    /// False when macOS privacy protection hid folders such as the Trash from
    /// the scan, which can leave a very large amount unexplained.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_disk_access: Option<bool>,
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
    if !path.exists() {
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
        .filter(|p| p.is_dir())
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
fn dependency_folders(roots: &[PathBuf]) -> (Sized, Sized) {
    let (mut node, mut venv) = (Vec::new(), Vec::new());
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
            if name == "node_modules" {
                node.push((e.path().to_path_buf(), disk_usage(e.path())));
                it.skip_current_dir();
            } else if name == ".git" || name == "Library" || name == ".Trash" {
                it.skip_current_dir();
            } else if e.path().join("pyvenv.cfg").is_file() {
                venv.push((e.path().to_path_buf(), disk_usage(e.path())));
                it.skip_current_dir();
            }
        }
    }
    (node, venv)
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
    fixed(&mut out, h.join("Library/Application Support/MobileSync/Backup"), "iphone-backups", "iPhone and iPad backups",
        "Full backups of devices made on this Mac. They can be very large, and old ones are easy to forget.",
        "In Finder, select your device in the sidebar and choose Manage Backups to delete old ones.");

    let (node, venv) = dependency_folders(&project_roots(&h));
    grouped(&mut out, node, "node-modules", "JavaScript project dependencies",
        "Each JavaScript project keeps a node_modules folder of the libraries it uses. They add up quickly across many projects.",
        "Delete node_modules in projects you are not working on. Running `npm install` in a project brings it back.");
    grouped(&mut out, venv, "python-venvs", "Python virtual environments",
        "Each Python project can have its own environment holding the libraries it uses.",
        "Delete environments for projects you are not working on. They can be recreated from the project's requirements.");

    if !is_running("WhatsApp") {
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
                });
            }
        }
    }

    out.sort_by_key(|d| std::cmp::Reverse(d.bytes));
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
    }
}
