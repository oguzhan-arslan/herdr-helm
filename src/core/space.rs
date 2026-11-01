//! The space record (<space>/.herdr/space.json), and the repos of a space or a folder.

use super::config::{config, git_repos, tasks_root, valid_space_name, Repos};
use crate::fail;
use crate::util::{expand, git_maybe, locked, write_json};
use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The PRs linked to a space: "prs", or the single "pr" of older PR spaces.
pub fn prs_of(rec: &Value) -> Vec<Value> {
    if let Some(prs) = rec["prs"].as_array() {
        return prs.clone();
    }
    if rec["pr"].is_object() {
        return vec![rec["pr"].clone()];
    }
    Vec::new()
}

pub fn record_path(space_dir: &Path) -> PathBuf {
    space_dir.join(".herdr").join("space.json")
}

fn read_record(space_dir: &Path) -> Result<Value> {
    let path = record_path(space_dir);
    let text = std::fs::read_to_string(&path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| anyhow!("{} is not JSON: {e}", path.display()))
}

/// The space by name, else the space that holds the folder this program runs in. A name must
/// be a valid space name, so `--space ../x` or an absolute path never reaches another folder.
pub fn load_space(c: &Value, name: Option<&str>) -> Result<(Value, PathBuf)> {
    let d = match name.filter(|n| !n.is_empty()) {
        Some(name) if !valid_space_name(name) => fail!("'{name}' is not a space name: 1-23 of a-z 0-9 -."),
        Some(name) => tasks_root(c).join(name),
        None => space_of_cwd()?,
    };
    if !record_path(&d).exists() {
        fail!("No space at {}", d.display());
    }
    Ok((read_record(&d)?, d))
}

fn space_of_cwd() -> Result<PathBuf> {
    let cwd = std::env::current_dir()?;
    let mut d = cwd.canonicalize().unwrap_or(cwd);
    while !record_path(&d).exists() {
        match d.parent() {
            Some(parent) => d = parent.to_path_buf(),
            None => fail!("Not inside a space folder. Give the space with --space <name>."),
        }
    }
    Ok(d)
}

pub fn save_space(rec: &Value, space_dir: &Path) -> Result<()> {
    write_json(&record_path(space_dir), rec)
}

/// Changes the record of a space under its lock: reads the record that is on disk now, gives it
/// to `change`, and writes it back. Returns the new record and what `change` returned. Use it
/// for every change of a record that exists, so that a change by another command is not lost.
pub fn update_space<T>(space_dir: &Path, change: impl FnOnce(&mut Value) -> Result<T>) -> Result<(Value, T)> {
    locked(&record_path(space_dir), || {
        let mut rec = read_record(space_dir)?;
        let result = change(&mut rec)?;
        save_space(&rec, space_dir)?;
        Ok((rec, result))
    })
}

/// Every space in the tasks folder: (its record, its folder), in the order of the folder names.
pub fn all_spaces(c: &Value) -> Result<Vec<(Value, PathBuf)>> {
    let Ok(entries) = std::fs::read_dir(tasks_root(c)) else { return Ok(Vec::new()) };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|d| record_path(d).exists()).collect();
    dirs.sort();
    dirs.into_iter().map(|d| Ok((read_record(&d)?, d))).collect()
}

/// The checkouts of a space whose folder is there: (folder name, path).
fn checkouts_of(rec: &Value) -> Repos {
    rec["checkouts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|k| Some((k["folder"].as_str()?.to_string(), PathBuf::from(k["path"].as_str()?))))
        .filter(|(_, p)| p.exists())
        .collect()
}

/// The repos to show for a Herdr space: a project space's checkouts, else the repo at the
/// folder, else the repos directly under it. Returns the space name too (or None).
pub fn space_repos(space: Option<&str>, herdr_space: Option<&str>, folder: Option<&str>) -> Result<(Option<String>, Repos)> {
    let c = config()?;
    if let Some(id) = herdr_space.filter(|h| !h.is_empty()) {
        for (rec, _) in all_spaces(&c)? {
            if rec["herdr_space"].as_str() == Some(id) {
                return Ok((rec["name"].as_str().map(String::from), checkouts_of(&rec)));
            }
        }
    }
    if let Some(name) = space.filter(|s| !s.is_empty()) {
        let (rec, _) = load_space(&c, Some(name))?;
        return Ok((rec["name"].as_str().map(String::from), checkouts_of(&rec)));
    }
    if let Some(folder) = folder.filter(|f| !f.is_empty()) {
        let folder = expand(folder);
        let top =
            if folder.is_dir() { git_maybe(&folder, &["rev-parse", "--show-toplevel"], false).unwrap_or_default() } else { String::new() };
        if top.is_empty() {
            return Ok((None, git_repos(&folder)));
        }
        let top = PathBuf::from(top);
        let name = top.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        return Ok((None, vec![(name, top)]));
    }
    Ok((None, Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prs_of_reads_both_forms() {
        assert_eq!(prs_of(&json!({ "prs": [{ "number": 1 }] })), vec![json!({ "number": 1 })]);
        assert_eq!(prs_of(&json!({ "pr": { "number": 2 } })), vec![json!({ "number": 2 })]);
        assert!(prs_of(&json!({ "pr": null })).is_empty());
        assert!(prs_of(&json!({})).is_empty());
    }

    #[test]
    fn record_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("x");
        let rec = json!({ "name": "x", "project": "p", "checkouts": [] });
        save_space(&rec, &dir).unwrap();
        assert_eq!(read_record(&dir).unwrap(), rec);
        let c = json!({ "tasks_root": tmp.path() });
        assert_eq!(load_space(&c, Some("x")).unwrap().0, rec);
        assert!(load_space(&c, Some("no-such-space-here")).is_err());
    }

    #[test]
    fn a_space_name_cannot_leave_the_tasks_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let (tasks, outside) = (tmp.path().join("tasks"), tmp.path().join("outside"));
        save_space(&json!({ "name": "outside" }), &outside).unwrap();
        std::fs::create_dir_all(&tasks).unwrap();
        let c = json!({ "tasks_root": tasks });
        for name in ["../outside", outside.to_str().unwrap(), "a/b", ".."] {
            let e = load_space(&c, Some(name)).unwrap_err().to_string();
            assert!(e.contains("is not a space name"), "{name}: {e}");
        }
    }

    #[test]
    fn update_space_reads_the_record_again() {
        let dir = tempfile::tempdir().unwrap();
        save_space(&json!({ "name": "x", "title": "old" }), dir.path()).unwrap();
        // Another command changes the record after this one read it.
        save_space(&json!({ "name": "x", "title": "old", "herdr_space": "s1" }), dir.path()).unwrap();
        let (rec, n) = update_space(dir.path(), |r| {
            r["title"] = json!("new");
            Ok(7)
        })
        .unwrap();
        assert_eq!(n, 7);
        assert_eq!(rec, json!({ "name": "x", "title": "new", "herdr_space": "s1" }));
        assert_eq!(read_record(dir.path()).unwrap(), rec);
    }
}
