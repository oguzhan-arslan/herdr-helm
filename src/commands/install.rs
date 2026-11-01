//! install (and update) and uninstall: puts Helm into a Herdr home, or takes it out. Safe to run
//! again and again; it keeps what the person changed. What it does in the Herdr home:
//!
//!   mods/helm-*          replaced with the repo's (Herdr does not reload a mod behind a symlink);
//!                        the files Herdr writes into a mod folder stay
//!   layouts/helm.json    the repo's structure; the panel state, the tiles and root look keys stay
//!   helm/bin/herdr-helm  this program; ~/.local/bin/herdr-helm links to it
//!   helm/templates/      the CLAUDE.md and the built-in skills of a space
//!   helm/state/          Helm's own files
//!   helm/install.json    what the last install did
//!   keys.json            only the "layout:helm" and "mod:helm-spaces" sections: a key the person
//!                        changed or removed stays; a key that still has its old default follows
//! projects.json is never touched.

use crate::cli::*;
use crate::paths::{app_home, helm_dir};
use crate::util::{home, read_json, short, tool};
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// mods/_shared: files every Helm mod gets, not a mod itself.
const SHARED: &str = "_shared";
/// The Herdr mods that Helm uses: their icons (workspace-chrome, and pane for the tile header),
/// the worktree dialog, and the new agent menu.
const HERDR_MODS: [&str; 4] = ["workspace-chrome", "pane", "worktree", "new-menu"];
const KEY_SECTIONS: [&str; 2] = ["layout:helm", "mod:helm-spaces"];

/// Files that Herdr writes into every mod folder: an update keeps them.
fn herdr_file(name: &str) -> bool {
    name == "gpui-kit.d.ts" || name == "jsconfig.json" || name.ends_with(".error")
}

/// The changes of one run; a dry run only lists them.
struct Run {
    dry: bool,
    changes: Vec<String>,
}

impl Run {
    fn note(&mut self, text: String) {
        println!("{}{}", if self.dry { "would " } else { "" }, text);
        self.changes.push(text);
    }

    fn copy(&mut self, src: &Path, dst: &Path) -> Result<()> {
        if dst.is_file() && fs::read(dst).ok() == fs::read(src).ok() {
            return Ok(());
        }
        self.note(format!("{} {}", if dst.exists() { "update" } else { "add" }, short(dst)));
        if !self.dry {
            replace_file(src, dst)?;
        }
        Ok(())
    }

    fn remove(&mut self, path: &Path) -> Result<()> {
        if !path.exists() && !path.is_symlink() {
            return Ok(());
        }
        self.note(format!("remove {}", short(path)));
        if !self.dry {
            if path.is_dir() && !path.is_symlink() {
                fs::remove_dir_all(path)?
            } else {
                fs::remove_file(path)?
            }
        }
        Ok(())
    }

    fn write_json(&mut self, path: &Path, data: &Value, why: &str) -> Result<()> {
        if read_json(path, Value::Null) == *data {
            return Ok(());
        }
        self.note(format!("{why}: {}", short(path)));
        if !self.dry {
            crate::util::write_json(path, data)?;
        }
        Ok(())
    }

    fn link(&mut self, link: &Path, target: &Path) -> Result<()> {
        if fs::read_link(link).ok().as_deref() == Some(target) {
            return Ok(());
        }
        if link.exists() && !link.is_symlink() {
            println!("note: {} is a file, not a link; left as it is.", short(link));
            return Ok(());
        }
        self.note(format!("link {} -> {}", short(link), short(target)));
        if !self.dry {
            if let Some(dir) = link.parent() {
                fs::create_dir_all(dir)?;
            }
            if link.is_symlink() {
                fs::remove_file(link)?;
            }
            std::os::unix::fs::symlink(target, link)?;
        }
        Ok(())
    }
}

/// Copies `src` to a temporary file next to `dst`, then renames it over `dst`. A program that
/// runs from `dst` keeps its old file, and macOS does not see a changed signed program.
fn replace_file(src: &Path, dst: &Path) -> Result<()> {
    let dir = dst.parent().ok_or_else(|| anyhow!("{} has no folder", dst.display()))?;
    fs::create_dir_all(dir)?;
    let name = dst.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let done = fs::copy(src, &tmp).and_then(|_| fs::rename(&tmp, dst));
    if done.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    done.map_err(|e| anyhow!("copy {} to {}: {e}", short(src), short(dst)))
}

/// The files to install: --from, else the clone above this program (a `cargo build` puts it in
/// <repo>/target/<profile>/), else the files inside this program, in state/source.
fn repo_dir(from: Option<&str>) -> Result<PathBuf> {
    if let Some(f) = from {
        return Ok(crate::util::expand(f));
    }
    let exe = std::env::current_exe()?.canonicalize()?;
    if let Some(repo) = exe.ancestors().find(|d| d.join("mods").is_dir() && d.join("keys/helm.json").is_file()) {
        return Ok(repo.to_path_buf());
    }
    // No clone: the files inside this program (a release download, or an update).
    let dir = crate::paths::state_dir().join("source");
    if dir.exists() {
        fs::remove_dir_all(&dir)?;
    }
    crate::assets::extract(&dir)?;
    Ok(dir)
}

/// Every file under `dir`, relative, without .DS_Store and Herdr's own files.
fn files_of(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if name == ".DS_Store" || herdr_file(&name) {
                continue;
            }
            if p.is_dir() {
                stack.push(p)
            } else if let Ok(rel) = p.strip_prefix(dir) {
                found.push(rel.to_path_buf())
            }
        }
    }
    found.sort();
    found
}

/// `dst` becomes a copy of `src` plus the files of `extra` (a mod gets mods/_shared, because a
/// mod cannot import from another mod's folder); files that Herdr writes there stay.
fn sync_tree(run: &mut Run, src: &Path, extra: Option<&Path>, dst: &Path) -> Result<()> {
    let mut wanted = Vec::new();
    for dir in extra.into_iter().chain(std::iter::once(src)) {
        for rel in files_of(dir) {
            run.copy(&dir.join(&rel), &dst.join(&rel))?;
            wanted.push(rel);
        }
    }
    for rel in files_of(dst) {
        if !wanted.contains(&rel) {
            run.remove(&dst.join(rel))?;
        }
    }
    Ok(())
}

fn find<'a>(node: &'a Value, id: &str) -> Option<&'a Value> {
    if node["id"] == id {
        return Some(node);
    }
    node["children"].as_array()?.iter().find_map(|c| find(c, id))
}

fn find_mut<'a>(node: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    if node["id"] == id {
        return Some(node);
    }
    node.get_mut("children")?.as_array_mut()?.iter_mut().find_map(|c| find_mut(c, id))
}

/// The repo's layout, with what Herdr and the person changed in the installed one: the panel's
/// state and size, the grid's tiles (the tiler writes them), and keys added on the root.
pub fn merge_layout(repo: &Value, installed: &Value) -> Value {
    let mut new = repo.clone();
    if installed["id"] != "root" {
        return new;
    }
    if let (Some(n), Some(o)) = (new.as_object_mut(), installed.as_object()) {
        for (k, v) in o {
            if !n.contains_key(k) {
                n.insert(k.clone(), v.clone());
            }
        }
    }
    for (id, keys) in [("panel", &["hidden", "active", "size"][..]), ("grid", &["children"][..])] {
        let Some(old) = find(installed, id).cloned() else { continue };
        let Some(node) = find_mut(&mut new, id) else { continue };
        for k in keys {
            if !old[*k].is_null() {
                node[*k] = old[*k].clone();
            }
        }
    }
    new
}

/// Merges the repo's Helm keys into the person's keys. `before`: the keys installed last time, to
/// tell a default from their own: a key that still has its old default follows the repo, a key
/// they changed or removed stays as they left it, and a dropped default goes unless they changed it.
pub fn merge_keys(mine: &Value, repo: &Value, before: &Value) -> Value {
    let mut mine = mine.clone();
    let mut sections: Vec<String> = repo.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    for k in before.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default() {
        if !sections.contains(&k) {
            sections.push(k);
        }
    }
    let empty = Map::new();
    for section in sections {
        let new = repo[&section].as_object().unwrap_or(&empty);
        let old = before[&section].as_object().unwrap_or(&empty);
        let mut have = mine[&section].as_object().cloned().unwrap_or_default();
        for (key, action) in new {
            match have.get(key) {
                None if !old.contains_key(key) => {
                    have.insert(key.clone(), action.clone());
                }
                Some(v) if Some(v) == old.get(key).or(Some(action)) => {
                    have.insert(key.clone(), action.clone());
                }
                _ => {}
            }
        }
        for (key, action) in old {
            if !new.contains_key(key) && have.get(key) == Some(action) {
                have.remove(key);
            }
        }
        if let Some(m) = mine.as_object_mut() {
            if have.is_empty() {
                m.remove(&section);
            } else {
                m.insert(section, Value::Object(have));
            }
        }
    }
    mine
}

fn version(repo: &Path) -> String {
    let v = crate::util::git_maybe(repo, &["describe", "--always", "--dirty", "--tags"], false).unwrap_or_default();
    if v.is_empty() {
        env!("CARGO_PKG_VERSION").to_string()
    } else {
        v
    }
}

fn shown_layout() -> Option<String> {
    let out = crate::util::exec(&tool("herdr-app"), &["status"], None, None).ok()?;
    serde_json::from_str::<Value>(&out.stdout).ok()?["layout"].as_str().map(String::from)
}

pub fn cmd_install(_c: &Ctx, a: &InstallArgs) -> Result<()> {
    let repo = repo_dir(a.from.as_deref())?;
    let home_dir = app_home();
    // Every precondition first: a run that stops half way leaves a half install.
    if !home_dir.is_dir() {
        return Err(anyhow!("no Herdr home at {}: install Herdr and start it once (or set HERDR_APP_HOME)", short(&home_dir)));
    }
    let keys_path = home_dir.join("keys.json");
    let mine = read_json(&keys_path, Value::Null);
    if mine.is_null() {
        return Err(anyhow!("no {}: start Herdr once, so that it writes its keys, then run this again", short(&keys_path)));
    }
    let repo_layout = read_json(&repo.join("layouts/helm.json"), Value::Null);
    let repo_keys = read_json(&repo.join("keys/helm.json"), Value::Null);
    if repo_layout.is_null() || repo_keys.is_null() || !repo.join("templates/CLAUDE.md").is_file() {
        return Err(anyhow!("{} is not a complete Helm repo", short(&repo)));
    }
    for (program, why) in [("git", "spaces are git worktrees"), ("hc", "Herdr's terminals")] {
        if !Path::new(&tool(program)).is_file() {
            println!("note: no `{program}`: {why} need it.");
        }
    }
    for m in HERDR_MODS {
        if !home_dir.join("mods").join(m).is_dir() {
            println!("note: no Herdr mod `{m}` in {}: Helm uses it. Update Herdr to get it back.", short(&home_dir.join("mods")));
        }
    }

    let mut run = Run { dry: a.dry_run, changes: Vec::new() };
    let state_file = helm_dir().join("install.json");
    let state = read_json(&state_file, json!({}));
    let first = state.as_object().is_none_or(|o| o.is_empty());

    // The mods.
    let shared = repo.join("mods").join(SHARED);
    let mut names: Vec<String> = fs::read_dir(repo.join("mods"))?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != SHARED)
        .collect();
    names.sort();
    for name in &names {
        sync_tree(&mut run, &repo.join("mods").join(name), Some(&shared), &home_dir.join("mods").join(name))?;
    }
    for g in recorded_mods(&state) {
        if !names.contains(&g) {
            run.remove(&home_dir.join("mods").join(g))?;
        }
    }

    // This program, the templates, and the links.
    let bin = helm_dir().join("bin/herdr-helm");
    let me = std::env::current_exe()?.canonicalize()?;
    if me != bin.canonicalize().unwrap_or_default() {
        run.copy(&me, &bin)?;
    }
    sync_tree(&mut run, &repo.join("templates"), None, &helm_dir().join("templates"))?;
    let local = home().join(".local/bin");
    run.link(&local.join("herdr-helm"), &bin)?;

    // The layout and the keys.
    let layout_path = home_dir.join("layouts/helm.json");
    let layout = merge_layout(&repo_layout, &read_json(&layout_path, Value::Null));
    run.write_json(&layout_path, &layout, "write the layout")?;
    let keys = merge_keys(&mine, &repo_keys, &state["keys"]);
    run.write_json(&keys_path, &keys, "merge the Helm keys into")?;

    let v = version(&repo);
    if !run.dry {
        crate::util::write_json(&state_file, &json!({ "version": v, "repo": repo, "mods": names, "keys": repo_keys }))?;
        // The CLAUDE.md and the skills of every space, from the new templates.
        let _ = crate::util::exec(bin.to_str().unwrap_or("herdr-helm"), &["refresh"], None, None);
        if first && crate::core::herdr::herdr("layout.switch", &json!({ "name": "helm" })).is_ok() {
            println!("Herdr now shows the helm layout.");
        }
    }
    let summary = if run.changes.is_empty() { "Nothing to change".to_string() } else { format!("{} changes", run.changes.len()) };
    println!("{summary}{}. Helm {v}.", if run.dry { " (dry run)" } else { "" });
    Ok(())
}

/// A name that install.json can give for a Helm mod. Any other name, such as "" or "../x",
/// would remove a folder that is not a Helm mod.
fn helm_mod_name(name: &str) -> bool {
    name.strip_prefix("helm-")
        .is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
}

/// The Helm mods that the last install recorded, without the names that are not Helm mod names.
fn recorded_mods(state: &Value) -> Vec<String> {
    state["mods"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|n| helm_mod_name(n)).map(String::from).collect()
}

pub fn cmd_uninstall(_c: &Ctx, a: &InstallArgs) -> Result<()> {
    let mut run = Run { dry: a.dry_run, changes: Vec::new() };
    let home_dir = app_home();
    let state = read_json(&helm_dir().join("install.json"), json!({}));
    if !run.dry && shown_layout().as_deref() == Some("helm") {
        let _ = crate::core::herdr::herdr("layout.switch", &json!({ "name": "main" }));
    }
    // Without install.json, only the mods that this program installs: never another helm-* mod.
    let mut names = recorded_mods(&state);
    if names.is_empty() {
        names = crate::assets::mod_names();
    }
    for name in names {
        run.remove(&home_dir.join("mods").join(name))?;
    }
    run.remove(&home_dir.join("layouts/helm.json"))?;
    let keys_path = home_dir.join("keys.json");
    let mut keys = read_json(&keys_path, Value::Null);
    if let Some(m) = keys.as_object_mut() {
        for s in KEY_SECTIONS {
            m.remove(s);
        }
        run.write_json(&keys_path, &keys, "remove the Helm keys from")?;
    }
    let link = home().join(".local/bin/herdr-helm");
    if fs::read_link(&link).is_ok_and(|t| t.starts_with(helm_dir())) {
        run.remove(&link)?;
    }
    // The program itself goes last: this may be it.
    run.remove(&helm_dir())?;
    println!("Removed Helm{}. projects.json and the spaces in the tasks folder stay.", if run.dry { " (dry run)" } else { "" });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_helm_mod_names_are_removed() {
        let state = json!({ "mods": ["helm-bar", "", "helm-", "../x", "helm-a/b", "other", "HELM-x", "helm-x2", 5] });
        assert_eq!(recorded_mods(&state), ["helm-bar", "helm-x2"]);
        assert!(recorded_mods(&json!({})).is_empty());
    }

    #[test]
    fn keys_follow_the_repo_but_keep_the_person_s_own() {
        let before = json!({ "layout:helm": { "cmd-k": "menu", "cmd-u": "usage", "cmd-g": "github", "cmd-e": "files" } });
        let repo = json!({ "layout:helm": { "cmd-k": "menu", "cmd-u": "usage", "cmd-g": "github2", "cmd-y": "new" } });
        let mine = json!({ "global": { "cmd-q": "quit" }, "layout:helm": { "cmd-k": "help", "cmd-g": "github", "cmd-e": "files" } });
        let got = merge_keys(&mine, &repo, &before);
        let h = &got["layout:helm"];
        assert_eq!(h["cmd-k"], "help"); // changed by the person
        assert!(h.get("cmd-u").is_none()); // removed by the person
        assert_eq!(h["cmd-g"], "github2"); // an old default follows the repo
        assert_eq!(h["cmd-y"], "new"); // a new default arrives
        assert!(h.get("cmd-e").is_none()); // a dropped default goes
        assert_eq!(got["global"]["cmd-q"], "quit");
    }

    #[test]
    fn first_install_adds_every_key() {
        let got = merge_keys(&json!({ "global": {} }), &json!({ "mod:helm-spaces": { "alt-tab": "next" } }), &Value::Null);
        assert_eq!(got["mod:helm-spaces"]["alt-tab"], "next");
    }

    #[test]
    fn copy_puts_a_new_file_in_place() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("new"), dir.path().join("bin/herdr-helm"));
        fs::write(&src, "new").unwrap();
        fs::create_dir_all(dst.parent().unwrap()).unwrap();
        fs::write(&dst, "old").unwrap();
        let before = fs::metadata(&dst).unwrap().ino();
        let mut run = Run { dry: false, changes: Vec::new() };
        run.copy(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "new");
        // A rename gives the target a new file. Writing into the old file would keep its inode.
        assert_ne!(fs::metadata(&dst).unwrap().ino(), before);
        assert_eq!(fs::read_dir(dst.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn layout_keeps_the_panel_tiles_and_look() {
        let repo = json!({ "id": "root", "children": [ { "id": "grid", "children": [] }, { "id": "panel", "hidden": true, "size": { "px": 368 } } ] });
        let installed = json!({ "id": "root", "theme": "graphite", "children": [ { "id": "grid", "children": [ { "id": "cell-main" } ] }, { "id": "panel", "hidden": false, "size": { "px": 500 } } ] });
        let got = merge_layout(&repo, &installed);
        assert_eq!(got["theme"], "graphite");
        assert_eq!(got["children"][0]["children"][0]["id"], "cell-main");
        assert_eq!(got["children"][1]["hidden"], false);
        assert_eq!(got["children"][1]["size"]["px"], 500);
    }
}
