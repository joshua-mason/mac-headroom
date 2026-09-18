//! Turning "this should be a built-in" into a suggestion someone can send.
//!
//! The built-in cleaners only grow if people who find something new tell us
//! about it, and the useful part of that is exactly what they had to work out:
//! where the files are and why removing them is safe. This fills in what the
//! tool already knows and opens a pre-filled GitHub issue. Nothing is sent: the
//! person reads it and decides whether to press Submit.

use crate::cleaners::{self, Cleaner, Source};
use crate::util::{disk_usage, expand, human, tilde, url_encode};
use serde::Serialize;
use std::path::PathBuf;

pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const TEMPLATE: &str = "suggest-cleaner.yml";

#[derive(Serialize, Default)]
pub struct Suggestion {
    pub title: String,
    pub what: String,
    pub locations: String,
    pub size: String,
    pub why_safe: String,
    pub config: String,
    pub url: String,
}

impl Suggestion {
    fn finish(mut self) -> Self {
        let mut url = format!("{REPOSITORY}/issues/new?template={TEMPLATE}");
        for (k, v) in [
            ("title", &self.title),
            ("what", &self.what),
            ("locations", &self.locations),
            ("size", &self.size),
            ("why_safe", &self.why_safe),
            ("config", &self.config),
        ] {
            if !v.is_empty() {
                url.push_str(&format!("&{k}={}", url_encode(v)));
            }
        }
        self.url = url;
        self
    }
}

fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// A cleaner as the config entry someone would paste into their own file.
fn config_entry(c: &Cleaner) -> String {
    let mut t = String::from("[[cleaner]]\n");
    t += &format!("name = {}\n", toml_string(&c.name));
    t += &format!("summary = {}\n", toml_string(&c.summary));
    t += &format!("why_safe = {}\n", toml_string(&c.why_safe));
    let list = |v: &[String]| {
        v.iter()
            .map(|x| toml_string(x))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if !c.paths.is_empty() {
        t += &format!("paths = [{}]\n", list(&c.paths));
    }
    if !c.command.is_empty() {
        t += &format!("command = [{}]\n", list(&c.command));
    }
    if let Some(n) = c.keep_newest {
        t += &format!("keep_newest = {n}\n");
    }
    if let Some(d) = c.older_than_days {
        t += &format!("older_than_days = {d}\n");
    }
    if let Some(app) = &c.skip_if_running {
        t += &format!("skip_if_running = {}\n", toml_string(app));
    }
    t
}

/// Build a suggestion from one of the person's own cleaners, a path, or nothing.
pub fn build(target: Option<&str>) -> Result<Suggestion, String> {
    let Some(target) = target else {
        return Ok(Suggestion::default().finish());
    };
    let all = crate::config::all_cleaners()?;
    if let Some(c) = cleaners::find(&all, target) {
        if c.source == Source::Builtin {
            return Err(format!(
                "{target} is already built in. To suggest a change to it, open an issue at {REPOSITORY}/issues"
            ));
        }
        let size = cleaners::estimate(c)
            .bytes
            .filter(|b| *b > 0)
            .map(human)
            .unwrap_or_default();
        return Ok(Suggestion {
            title: format!("Cleaner: {}", c.summary),
            what: c.summary.clone(),
            locations: c
                .paths
                .iter()
                .cloned()
                .chain((!c.command.is_empty()).then(|| format!("(runs: {})", c.command.join(" "))))
                .collect::<Vec<_>>()
                .join("\n"),
            size,
            why_safe: c.why_safe.clone(),
            config: config_entry(c),
            url: String::new(),
        }
        .finish());
    }
    let path = PathBuf::from(expand(target));
    if path.exists() {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| tilde(&path));
        return Ok(Suggestion {
            title: format!("Cleaner: {name}"),
            locations: tilde(&path),
            size: Some(disk_usage(&path))
                .filter(|b| *b > 0)
                .map(human)
                .unwrap_or_default(),
            ..Default::default()
        }
        .finish());
    }
    Err(format!(
        "{target} is neither one of your cleaners nor a path that exists here. \
         Run `mac-headroom suggest` on its own for a blank suggestion."
    ))
}

pub fn print_text(s: &Suggestion) {
    println!(
        "A suggestion is ready to review on GitHub. Nothing is sent until you press Submit there."
    );
    if !s.locations.is_empty() {
        println!("\nPre-filled from this Mac:");
        println!("  where    {}", s.locations.replace('\n', "\n           "));
        if !s.size.is_empty() {
            println!("  size     {}", s.size);
        }
        if !s.why_safe.is_empty() {
            println!("  why safe {}", s.why_safe);
        }
    }
    println!("\nThe most useful thing to add is how you know it is safe to remove.");
    println!("\n{}", s.url);
}
