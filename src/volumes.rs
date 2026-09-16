//! What else is sharing this disk.
//!
//! A directory walk only ever sees one filesystem, so everything that is not
//! the data volume is invisible to it: the sealed system volume, swap, Preboot,
//! Recovery, and any mounted disk image. Those are not curiosities. On this
//! kind of Mac they are tens of gigabytes, and they are most of the difference
//! between what a scan can find and what the disk says is gone.

use crate::util::{human, stdout_of};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
pub struct Volume {
    pub name: String,
    pub device: String,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mount: Option<String>,
    /// True for the volume a home directory lives on, which is the one every
    /// scan covers. The rest is what a scan can never reach.
    pub is_data: bool,
}

#[derive(Serialize)]
pub struct DiskImage {
    /// The file the mounted image is read from. This is what costs real space.
    pub backing_path: String,
    pub backing_bytes: u64,
    /// Whether that file is somewhere a scan would already have counted.
    pub backing_under_home: bool,
}

#[derive(Serialize, Default)]
pub struct Volumes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    pub volumes: Vec<Volume>,
    pub images: Vec<DiskImage>,
}

/// "Part of Whole: disk3" for the volume holding user data.
fn container() -> Option<String> {
    let info = stdout_of("diskutil", &["info", "/System/Volumes/Data"]);
    info.lines()
        .find(|l| l.contains("Part of Whole:"))
        .and_then(|l| l.split(':').nth(1))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Mount points by device, from df, so a volume can name where it appears.
fn mounts() -> Vec<(String, String)> {
    stdout_of("df", &["-k"])
        .lines()
        .skip(1)
        .filter_map(|l| {
            let dev = l.split_whitespace().next()?;
            let mount = l.split_whitespace().skip(8).collect::<Vec<_>>().join(" ");
            let dev = dev.strip_prefix("/dev/")?;
            (!mount.is_empty()).then(|| (dev.to_string(), mount))
        })
        .collect()
}

/// Volumes sharing the container, with what each has actually consumed.
/// Parsed from `diskutil apfs list`, which lists volumes that are not mounted
/// (Recovery) as well as those that are.
fn container_volumes(container: &str, mounts: &[(String, String)]) -> Vec<Volume> {
    let text = stdout_of("diskutil", &["apfs", "list", container]);
    let mut out: Vec<Volume> = Vec::new();
    let mut device = String::new();
    let mut name = String::new();
    for line in text.lines() {
        let t = line.trim_start_matches(['|', '+', '-', '>', ' ']);
        if let Some(rest) = t.strip_prefix("Volume ") {
            // Flush a volume that had no size line, then start the next.
            device = rest.split_whitespace().next().unwrap_or("").to_string();
            name.clear();
        } else if let Some(rest) = t.strip_prefix("Name:") {
            name = rest
                .trim()
                .split(" (Case-")
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
        } else if let Some(rest) = t.strip_prefix("Capacity Consumed:") {
            let bytes = rest
                .split_whitespace()
                .next()
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0);
            if !device.is_empty() {
                // A volume can be mounted at its own device or a snapshot of it
                // (the system volume mounts as diskNsMsK at "/").
                let mount = mounts
                    .iter()
                    .find(|(d, _)| *d == device || d.starts_with(&format!("{device}s")))
                    .map(|(_, m)| m.clone());
                let is_data = mount.as_deref() == Some("/System/Volumes/Data");
                out.push(Volume {
                    name: std::mem::take(&mut name),
                    device: std::mem::take(&mut device),
                    bytes,
                    mount,
                    is_data,
                });
            }
        }
    }
    out.sort_by_key(|v| std::cmp::Reverse(v.bytes));
    out
}

/// Attached disk images, and the file each one is actually read from. The mount
/// is a view; the backing file is the space.
fn images() -> Vec<DiskImage> {
    let home = crate::util::home();
    let mut out = Vec::new();
    for line in stdout_of("hdiutil", &["info"]).lines() {
        let Some(rest) = line.strip_prefix("image-path") else {
            continue;
        };
        let path = rest.trim_start_matches([':', ' ']).trim();
        if path.is_empty() {
            continue;
        }
        out.push(DiskImage {
            backing_bytes: crate::util::disk_usage(Path::new(path)),
            backing_under_home: Path::new(path).starts_with(&home),
            backing_path: path.to_string(),
        });
    }
    out.sort_by_key(|i| std::cmp::Reverse(i.backing_bytes));
    out.dedup_by(|a, b| a.backing_path == b.backing_path);
    out
}

pub fn gather() -> Volumes {
    let Some(c) = container() else {
        return Volumes::default();
    };
    let m = mounts();
    Volumes {
        volumes: container_volumes(&c, &m),
        images: images(),
        container: Some(c),
    }
}

pub fn print_text(v: &Volumes) {
    match &v.container {
        None => {
            println!("Could not identify the container holding the data volume.");
            return;
        }
        Some(c) => println!("Container {c}"),
    }
    if v.volumes.is_empty() {
        println!("  no volumes reported");
    }
    for vol in &v.volumes {
        println!(
            "  {:>9}  {:<22} {}{}",
            human(vol.bytes),
            vol.name,
            vol.mount.as_deref().unwrap_or("not mounted"),
            if vol.is_data {
                "   <- the only one a scan sees"
            } else {
                ""
            }
        );
    }
    let hidden: u64 = v
        .volumes
        .iter()
        .filter(|x| !x.is_data)
        .map(|x| x.bytes)
        .sum();
    if hidden > 0 {
        println!(
            "\n{} is on volumes that share this disk but that no directory scan can reach.",
            human(hidden)
        );
    }
    if !v.images.is_empty() {
        println!("\nAttached disk images, and the file each is read from:");
        for i in &v.images {
            println!(
                "  {:>9}  {}{}",
                human(i.backing_bytes),
                i.backing_path,
                if i.backing_under_home {
                    ""
                } else {
                    "   (outside your home folder)"
                }
            );
        }
        println!("The mount is a view of that file. Only the file costs space.");
    }
}
