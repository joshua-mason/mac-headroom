//! Files an application downloaded and then lost track of.
//!
//! Some apps keep their media on disk and their record of it in a database. If
//! the database is replaced, as WhatsApp's is when you re-link the Mac as a
//! device, the files stay and nothing points at them any more. The app's own
//! storage screen cannot see them, because it reports what the database knows.
//!
//! There is no general way to detect this: every application tracks its data
//! differently, and guessing would eventually delete something real. So this is
//! deliberately specific. A spec names one app's media directory, its database,
//! and the columns that hold paths, and a file is only ever a candidate if the
//! database has been read successfully and does not mention it.

use crate::util::expand;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug)]
pub struct OrphanSpec {
    /// Directory holding both the media and the database, under `~/`.
    pub app_dir: &'static str,
    /// Media directory, relative to `app_dir`.
    pub media_dir: &'static str,
    /// SQLite database, relative to `app_dir`.
    pub db: &'static str,
    /// Columns holding paths are discovered from the schema rather than listed,
    /// so a column added by a later version of the app is picked up instead of
    /// making everything it references look orphaned. A column counts if its
    /// name contains this.
    pub path_column_marker: &'static str,
    /// Subdirectory names inside the media directory to leave alone.
    pub skip_dirs: &'static [&'static str],
    /// A query that must return a non-zero count. It proves the database was
    /// read rather than silently returning nothing.
    pub sanity_query: &'static str,
}

fn sqlite(db: &Path, query: &str) -> Result<Vec<String>, String> {
    let out = Command::new("/usr/bin/sqlite3")
        .arg(db)
        .arg(query)
        .output()
        .map_err(|e| format!("could not run sqlite3: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .filter(|l| !l.is_empty())
        .collect())
}

/// Copy the database and its write-ahead log somewhere private before reading.
/// Without the log, writes the app has not yet checkpointed are invisible, and
/// a file referenced only there would look orphaned.
fn stage_db(src: &Path) -> Result<(PathBuf, PathBuf), String> {
    // Unique per call, not per second: two runs at once must not share a
    // staging directory, or one will delete the database the other is reading.
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "mac-headroom-db-{}-{nanos}-{}",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join("db.sqlite");
    fs::copy(src, &dest).map_err(|e| format!("could not read {}: {e}", src.display()))?;
    for suffix in ["-wal", "-shm"] {
        let from = PathBuf::from(format!("{}{suffix}", src.display()));
        if from.exists() {
            let to = PathBuf::from(format!("{}{suffix}", dest.display()));
            fs::copy(&from, &to).map_err(|e| e.to_string())?;
        }
    }
    Ok((dir, dest))
}

/// Every file under the media directory that the database does not mention.
/// Any uncertainty is an error, never an empty result treated as "delete all".
pub fn find(spec: &OrphanSpec) -> Result<Vec<PathBuf>, String> {
    let app = PathBuf::from(expand(spec.app_dir));
    let media = app.join(spec.media_dir);
    let db = app.join(spec.db);
    if !media.is_dir() {
        return Ok(Vec::new());
    }
    if !db.is_file() {
        return Err(format!(
            "{} is not there, so nothing can be checked against it",
            db.display()
        ));
    }

    let (tmp, staged) = stage_db(&db)?;
    let result = (|| {
        let sane: i64 = sqlite(&staged, spec.sanity_query)?
            .first()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        if sane == 0 {
            return Err("the database read back empty, so nothing is safe to call orphaned".into());
        }
        // Ask the schema which columns can hold a path. Assuming a fixed list
        // would mean a column added by a future release is silently ignored,
        // and every file it references would be called an orphan.
        let tables = sqlite(
            &staged,
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        )?;
        let mut columns: Vec<(String, String)> = Vec::new();
        for table in &tables {
            for row in sqlite(&staged, &format!("PRAGMA table_info(\"{table}\")"))? {
                if let Some(col) = row.split('|').nth(1) {
                    if col.to_ascii_uppercase().contains(spec.path_column_marker) {
                        columns.push((table.clone(), col.to_string()));
                    }
                }
            }
        }
        if columns.is_empty() {
            return Err(format!(
                "no column in {} looks like it holds a path, so the schema is not what was expected",
                spec.db
            ));
        }

        let mut refs: HashSet<String> = HashSet::new();
        for (table, col) in &columns {
            let q = format!("SELECT \"{col}\" FROM \"{table}\" WHERE \"{col}\" IS NOT NULL");
            for row in sqlite(&staged, &q)? {
                let p = row.trim();
                if p.is_empty() {
                    continue;
                }
                // Match generously: a stored path may be relative to a
                // different root than the one being walked.
                refs.insert(p.to_string());
                refs.insert(p.trim_start_matches('/').to_string());
                if let Some(name) = Path::new(p).file_name() {
                    refs.insert(name.to_string_lossy().into_owned());
                }
            }
        }
        if refs.is_empty() {
            return Err("the database mentions no files at all, which is not believable".into());
        }

        let mut orphans = Vec::new();
        for entry in walkdir::WalkDir::new(&media)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                !e.file_type().is_dir()
                    || e.depth() == 0
                    || !spec
                        .skip_dirs
                        .iter()
                        .any(|d| e.file_name().to_string_lossy() == *d)
            })
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            let from_app = path
                .strip_prefix(&app)
                .map(|p| p.to_string_lossy().into_owned());
            let from_parent = path
                .strip_prefix(media.parent().unwrap_or(&app))
                .map(|p| p.to_string_lossy().into_owned());
            let known = name.as_deref().is_some_and(|n| refs.contains(n))
                || from_app.as_deref().is_ok_and(|p| refs.contains(p))
                || from_parent.as_deref().is_ok_and(|p| refs.contains(p));
            if !known {
                orphans.push(path.to_path_buf());
            }
        }
        orphans.sort();
        Ok(orphans)
    })();
    let _ = fs::remove_dir_all(&tmp);
    result
}

/// WhatsApp re-links as a device by replacing its database. The media it had
/// already downloaded stays behind with nothing left pointing at it.
pub const WHATSAPP: OrphanSpec = OrphanSpec {
    app_dir: "~/Library/Group Containers/group.net.whatsapp.WhatsApp.shared",
    media_dir: "Message/Media",
    db: "ChatStorage.sqlite",
    path_column_marker: "PATH",
    skip_dirs: &["Profile"],
    sanity_query: "SELECT COUNT(*) FROM ZWAMESSAGE",
};

#[cfg(test)]
mod tests {
    use super::*;

    fn make_db(dir: &Path, rows: &[&str]) -> PathBuf {
        let db = dir.join("t.sqlite");
        let mut sql = String::from(
            "CREATE TABLE m(p TEXT); CREATE TABLE msg(x INT); INSERT INTO msg VALUES(1);",
        );
        for r in rows {
            sql.push_str(&format!("INSERT INTO m VALUES('{r}');"));
        }
        Command::new("/usr/bin/sqlite3")
            .arg(&db)
            .arg(&sql)
            .output()
            .unwrap();
        db
    }

    fn spec(dir: &'static str) -> OrphanSpec {
        OrphanSpec {
            app_dir: dir,
            media_dir: "Media",
            db: "t.sqlite",
            path_column_marker: "P",
            skip_dirs: &["Profile"],
            sanity_query: "SELECT COUNT(*) FROM msg",
        }
    }

    #[test]
    fn keeps_referenced_and_protected_files() {
        let root = std::env::temp_dir().join(format!("mh-orph-{:?}", std::thread::current().id()));
        let _ = fs::remove_dir_all(&root);
        let media = root.join("Media/chat");
        fs::create_dir_all(&media).unwrap();
        fs::create_dir_all(root.join("Media/Profile")).unwrap();
        fs::write(media.join("keep.jpg"), b"a").unwrap();
        fs::write(media.join("drop.jpg"), b"b").unwrap();
        fs::write(root.join("Media/Profile/pic.jpg"), b"c").unwrap();
        make_db(&root, &["Media/chat/keep.jpg"]);

        let leaked: &'static str = Box::leak(root.to_string_lossy().into_owned().into_boxed_str());
        let found = find(&spec(leaked)).unwrap();
        assert_eq!(
            found,
            vec![media.join("drop.jpg")],
            "only the unreferenced file"
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_when_no_column_could_hold_a_path() {
        let root = std::env::temp_dir().join(format!("mh-orph3-{:?}", std::thread::current().id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Media/chat")).unwrap();
        fs::write(root.join("Media/chat/a.jpg"), b"a").unwrap();
        make_db(&root, &["Media/chat/a.jpg"]);
        let leaked: &'static str = Box::leak(root.to_string_lossy().into_owned().into_boxed_str());
        let mut sp = spec(leaked);
        sp.path_column_marker = "NOSUCHCOLUMN";
        assert!(find(&sp).is_err(), "an unexpected schema must refuse");
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn refuses_when_the_database_says_nothing() {
        let root = std::env::temp_dir().join(format!("mh-orph2-{:?}", std::thread::current().id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("Media/chat")).unwrap();
        fs::write(root.join("Media/chat/a.jpg"), b"a").unwrap();
        make_db(&root, &[]); // no rows: a failed read looks exactly like this
        let leaked: &'static str = Box::leak(root.to_string_lossy().into_owned().into_boxed_str());
        assert!(
            find(&spec(leaked)).is_err(),
            "must refuse rather than call everything an orphan"
        );
        fs::remove_dir_all(&root).unwrap();
    }
}
