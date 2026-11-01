//! The checkouts (git worktrees) of a space: add one, its state, its changes, remove it.

use super::config::{find_repo, inside_path, names, project, project_repos};
use super::{text, update_space};
use crate::fail;
use crate::util::{exec, git, git_maybe, has_ref, READ};
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `git -C <cwd> <args>` with stdout as it is (`status --porcelain` lines start with a space).
fn git_raw(cwd: &Path, args: &[&str]) -> Result<String> {
    let mut all = vec!["-C", cwd.to_str().unwrap_or(".")];
    all.extend_from_slice(args);
    let out = exec("git", &all, None, None)?;
    if !out.ok() {
        let why = if out.stderr.trim().is_empty() { out.stdout.trim() } else { out.stderr.trim() };
        fail!("git -C {} failed: {why}", cwd.display());
    }
    Ok(out.stdout)
}

/// READ followed by the command's own arguments.
fn read_args<'a>(args: &[&'a str]) -> Vec<&'a str> {
    READ.iter().copied().chain(args.iter().copied()).collect()
}

/// The label of an extra checkout, or None for the main checkout of a repo.
fn label_of(k: &Value) -> Option<&str> {
    text(k, "label")
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

fn default_base(repo: &Path) -> Result<String> {
    if let Some(r) = git_maybe(repo, &["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"], false).filter(|r| !r.is_empty()) {
        return Ok(r);
    }
    for b in ["origin/main", "origin/master", "main", "master"] {
        if has_ref(repo, b) {
            return Ok(b.to_string());
        }
    }
    fail!("No default branch found in {}", repo.display())
}

/// The project's base for new branches, when this repo has it. Else the repo's default.
pub(super) fn project_base(repo: &Path, p: &Value) -> Result<String> {
    match text(p, "base") {
        Some(base) if has_ref(repo, base) => Ok(base.to_string()),
        _ => default_base(repo),
    }
}

/// The primary (unlabeled) checkout of a repo in a space.
fn primary_of<'a>(rec: &'a Value, repo: &str) -> Option<&'a Value> {
    rec["checkouts"].as_array()?.iter().find(|k| k["repo"] == repo && label_of(k).is_none())
}

/// Adds a checkout of a repo to the space and saves the record. Returns the new entry of
/// "checkouts" with its "notes" (what went wrong in copy, link and setup). `rec` becomes the
/// record on disk, with the new checkout.
pub fn add_checkout(
    c: &Value,
    rec: &mut Value,
    space_dir: &Path,
    repo: &str,
    label: Option<&str>,
    branch: Option<&str>,
    fetch: bool,
) -> Result<Value> {
    let (k, notes) = make_checkout(c, rec, space_dir, repo, label, branch, fetch)?;
    match update_space(space_dir, |fresh| {
        push_checkout(fresh, &k);
        Ok(())
    }) {
        Ok((fresh, ())) => *rec = fresh,
        Err(e) => {
            undo_checkout(&k);
            return Err(e);
        }
    }
    Ok(with_notes(k, notes))
}

pub(super) fn push_checkout(rec: &mut Value, k: &Value) {
    if let Some(list) = rec["checkouts"].as_array_mut() {
        list.push(k.clone());
    } else {
        rec["checkouts"] = json!([k]);
    }
}

pub(super) fn with_notes(mut k: Value, notes: Vec<String>) -> Value {
    k["notes"] = json!(notes);
    k
}

/// Makes the worktree of a new checkout, but does not change the record: `new` makes the
/// checkouts of a space in parallel and adds each to the record under a lock.
pub(super) fn make_checkout(
    c: &Value,
    rec: &Value,
    space_dir: &Path,
    repo: &str,
    label: Option<&str>,
    branch: Option<&str>,
    fetch: bool,
) -> Result<(Value, Vec<String>)> {
    let project_name = str_of(rec, "project");
    let p = project(c, project_name)?;
    let repos = project_repos(p);
    let Some(src) = find_repo(&repos, repo) else {
        let known: Vec<&str> = repos.iter().map(|(n, _)| n.as_str()).collect();
        fail!("'{repo}' is not a repo of project {project_name}. Known: {}", known.join(", "));
    };
    let label = label.filter(|l| !l.is_empty());
    if label.is_some_and(|l| !super::valid_label(l)) {
        fail!("A label is 1-8 of a-z 0-9 -, and starts with a letter or digit.");
    }
    let settings = p["repos"].get(repo).cloned().unwrap_or_else(|| json!({}));
    let folder = label.map_or_else(|| repo.to_string(), |l| format!("{repo}.{l}"));
    if rec["checkouts"].as_array().into_iter().flatten().any(|k| k["folder"] == folder.as_str()) {
        fail!("The space already has a checkout '{folder}'.");
    }
    let name = str_of(rec, "name");
    // A PR's space checks out the PR's own branch (`branch`); other spaces use their name.
    let branch = branch.map(String::from).unwrap_or_else(|| label.map_or_else(|| name.to_string(), |l| format!("{name}-{l}")));
    let dest = space_dir.join(&folder);

    // Offline still works: then the base is what origin was at the last fetch.
    if fetch {
        git_maybe(src, &["fetch", "--quiet", "origin"], false);
    }
    let (base, made) = add_worktree(src, &dest, &branch, || {
        // An extra checkout starts from the space's own branch, so its work merges back there.
        match (label, primary_of(rec, repo)) {
            (Some(_), Some(primary)) => Ok(str_of(primary, "branch").to_string()),
            _ => text(&settings, "base").map(|b| Ok(b.to_string())).unwrap_or_else(|| project_base(src, p)),
        }
    })?;

    // Copied and linked files are local to the checkout: they never count as changes.
    let mut local = names(&settings["copy"]);
    local.extend(names(&settings["link"]));
    local.retain(|f| inside_path(f));
    let k = json!({
        "repo": repo, "label": label, "folder": folder, "branch": branch, "base": base,
        "path": dest.to_string_lossy(), "local": local, "made_branch": made,
    });
    // All or nothing: when a copy or a link fails, the worktree and its new branch go again.
    let copied = copy_files(src, &dest, &settings).and_then(|mut notes| {
        notes.extend(link_files(src, &dest, &settings)?);
        Ok(notes)
    });
    let mut notes = match copied {
        Ok(notes) => notes,
        Err(e) => {
            undo_checkout(&k);
            return Err(e);
        }
    };
    notes.extend(run_setup(src, &dest, &settings, name));
    Ok((k, notes))
}

/// Removes the worktree of a checkout that was just made, and its branch when the checkout
/// made that branch. Errors are left out: this runs after another error.
pub(super) fn undo_checkout(k: &Value) {
    let src = repo_source(k);
    let _ = remove_checkout_dir(k, true);
    if let Ok(src) = src {
        git_maybe(&src, &["worktree", "prune"], false);
        if made_branch(k) {
            git_maybe(&src, &["branch", "-D", str_of(k, "branch")], false);
        }
    }
}

/// True when `path` stays inside `base` after links are followed: the nearest folder of `path`
/// that exists is `base` or under it. Helm writes and removes the local files of a checkout only
/// there, also when a folder in the repo is a link to another place.
fn stays_inside(base: &Path, path: &Path) -> bool {
    let Ok(base) = base.canonicalize() else { return false };
    let mut dir = path.parent();
    while let Some(d) = dir {
        if let Ok(real) = d.canonicalize() {
            return real.starts_with(&base);
        }
        dir = d.parent();
    }
    false
}

/// The `copy` or `link` entries of a repo that Helm may use: paths inside the checkout. The
/// others become notes. `project update` refuses them, but an older projects.json can have them.
fn safe_entries(settings: &Value, key: &str, dest: &Path, notes: &mut Vec<String>) -> Vec<String> {
    let mut found = Vec::new();
    for f in names(&settings[key]) {
        if !inside_path(&f) || !stays_inside(dest, &dest.join(&f)) {
            notes.push(format!("{key}: {f} is not a path inside the checkout, left out"));
        } else {
            found.push(f);
        }
    }
    found
}

/// `git worktree add` of the branch: the local branch, else the remote one, else a new branch
/// from the base. Returns where the checkout starts from.
/// Adds the worktree on `branch`. Gives the base, and whether this made the branch: a local
/// branch that was there before is not Helm's, so Helm never deletes it.
fn add_worktree(src: &Path, dest: &Path, branch: &str, base: impl FnOnce() -> Result<String>) -> Result<(String, bool)> {
    let d = dest.to_string_lossy();
    if has_ref(src, &format!("refs/heads/{branch}")) {
        git(src, &["worktree", "add", &d, branch])?;
        return Ok((branch.to_string(), false));
    }
    let remote = format!("origin/{branch}");
    if has_ref(src, &format!("refs/remotes/{remote}")) {
        git(src, &["worktree", "add", "--track", "-b", branch, &d, &remote])?;
        return Ok((remote, true));
    }
    let base = base()?;
    git(src, &["worktree", "add", "--no-track", "-b", branch, &d, &base])?;
    Ok((base, true))
}

/// True when Helm made the local branch of the checkout. Only then may Helm delete it. A record
/// without "made_branch" is from before Helm kept this, so its branch stays.
pub fn made_branch(k: &Value) -> bool {
    k["made_branch"].as_bool().unwrap_or(false) && !k["external"].as_bool().unwrap_or(false)
}

fn copy_files(src: &Path, dest: &Path, settings: &Value) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    for f in safe_entries(settings, "copy", dest, &mut notes) {
        let (s, d) = (src.join(&f), dest.join(&f));
        if !s.exists() {
            notes.push(format!("copy: {f} is missing in {}", src.display()));
            continue;
        }
        // A link in the checkout could point out of it: a copy never writes through one.
        if d.is_symlink() {
            notes.push(format!("copy: {f} is a link in the checkout, left as it is"));
            continue;
        }
        if let Some(parent) = d.parent() {
            std::fs::create_dir_all(parent)?;
        }
        copy_tree(&s, &d)?;
    }
    Ok(notes)
}

fn link_files(src: &Path, dest: &Path, settings: &Value) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    for f in safe_entries(settings, "link", dest, &mut notes) {
        let (s, d) = (src.join(&f), dest.join(&f));
        if !s.exists() {
            notes.push(format!("link: {f} is missing in {}", src.display()));
            continue;
        }
        if d.symlink_metadata().is_ok() {
            notes.push(format!("link: {f} already exists in the checkout, left as it is"));
            continue;
        }
        if let Some(parent) = d.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::os::unix::fs::symlink(&s, &d)?;
    }
    Ok(notes)
}

/// The repo's setup command, in the new checkout. A failure is a note, not an error.
fn run_setup(src: &Path, dest: &Path, settings: &Value, space: &str) -> Vec<String> {
    let Some(setup) = text(settings, "setup") else { return Vec::new() };
    let out = std::process::Command::new("sh")
        .args(["-c", setup])
        .current_dir(dest)
        .env("HERDR_ROOT_PATH", src)
        .env("HERDR_WORKTREE_PATH", dest)
        .env("HERDR_SPACE", space)
        .stdin(std::process::Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => Vec::new(),
        Ok(o) => {
            let (stdout, stderr) = (String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            let why = if stderr.trim().is_empty() { stdout.trim() } else { stderr.trim() };
            vec![format!("setup failed (exit {}): {}", o.status.code().unwrap_or(-1), last_chars(why, 400))]
        }
        Err(e) => vec![format!("setup failed: {e}")],
    }
}

fn last_chars(s: &str, n: usize) -> &str {
    let count = s.chars().count();
    match s.char_indices().nth(count.saturating_sub(n)) {
        Some((i, _)) if count > n => &s[i..],
        _ => s,
    }
}

/// Copies a file, or a folder into a folder (what is there stays). Files with the same
/// content are not written again. Returns true when it wrote a file.
pub(super) fn copy_tree(src: &Path, dst: &Path) -> Result<bool> {
    if !src.is_dir() {
        if std::fs::read(dst).ok() == Some(std::fs::read(src)?) {
            return Ok(false);
        }
        std::fs::copy(src, dst).map_err(|e| anyhow!("copy {} to {}: {e}", src.display(), dst.display()))?;
        return Ok(true);
    }
    std::fs::create_dir_all(dst)?;
    let mut wrote = false;
    for entry in std::fs::read_dir(src)?.flatten() {
        wrote |= copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
    }
    Ok(wrote)
}

/// The lines of `status --porcelain` that are not the checkout's local (copied or linked) files.
fn changed_lines<'a>(porcelain: &'a str, local: &[String]) -> Vec<&'a str> {
    let local: Vec<&str> = local.iter().map(|f| f.trim_end_matches('/')).collect();
    porcelain
        .lines()
        .filter(|l| !l.is_empty())
        .filter(|l| {
            let path = l.get(3..).unwrap_or("");
            !local.iter().any(|f| path.trim_end_matches('/') == *f || path.starts_with(&format!("{f}/")))
        })
        .collect()
}

fn count(s: &str) -> Result<i64> {
    s.trim().parse().map_err(|_| anyhow!("not a number: {s}"))
}

/// The branch, changes and push state of a checkout. An extra checkout also says whether its
/// branch is merged into the space branch of its repo.
pub fn checkout_status(rec: &Value, k: &Value) -> Result<Value> {
    let path = PathBuf::from(str_of(k, "path"));
    let mut s = k.as_object().cloned().unwrap_or_default();
    if !path.exists() {
        s.insert("missing".into(), json!(true));
        return Ok(Value::Object(s));
    }
    let porcelain = git_raw(&path, &read_args(&["status", "--porcelain"]))?;
    let dirty = changed_lines(&porcelain, &names(&k["local"])).len();
    let upstream = git_maybe(&path, &["rev-parse", "--abbrev-ref", "@{u}"], false).filter(|u| !u.is_empty());
    let (mut ahead, mut behind) = (0, 0);
    if upstream.is_some() {
        let counts = git(&path, &["rev-list", "--left-right", "--count", "HEAD...@{u}"])?;
        let mut parts = counts.split_whitespace();
        ahead = count(parts.next().unwrap_or(""))?;
        behind = count(parts.next().unwrap_or(""))?;
    }
    let unpushed = count(&git(&path, &["rev-list", "--count", "HEAD", "--not", "--remotes"])?)?;
    s.insert("head".into(), json!(git(&path, &["rev-parse", "--abbrev-ref", "HEAD"])?));
    s.insert("dirty".into(), json!(dirty));
    s.insert("upstream".into(), json!(upstream));
    s.insert("ahead".into(), json!(ahead));
    s.insert("behind".into(), json!(behind));
    s.insert("unpushed".into(), json!(unpushed));
    if label_of(k).is_some() {
        let merged = primary_of(rec, str_of(k, "repo")).is_some_and(|primary| {
            let args = ["-C", str_of(k, "path"), "merge-base", "--is-ancestor", "HEAD", str_of(primary, "branch")];
            exec("git", &args, None, None).is_ok_and(|o| o.ok())
        });
        s.insert("merged".into(), json!(merged));
    }
    Ok(Value::Object(s))
}

/// The main checkout under the project root: it owns the worktree list of the repo.
pub fn repo_source(k: &Value) -> Result<PathBuf> {
    let common = git(Path::new(str_of(k, "path")), &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let common = PathBuf::from(common);
    Ok(common.parent().map(Path::to_path_buf).unwrap_or(common))
}

/// Removes the worktree of a checkout. Without `force`, git refuses a worktree with changes, so
/// this first makes sure that the only changes are the local (copied and linked) files. Only
/// then do these files go, and then the worktree. With `force`, git removes the worktree with
/// all its files. A local file goes only when it is inside the checkout.
pub fn remove_checkout_dir(k: &Value, force: bool) -> Result<()> {
    // An external checkout (another tool's worktree, linked into the space) is not ours:
    // only the link goes.
    if k["external"].as_bool().unwrap_or(false) {
        let link = PathBuf::from(str_of(k, "link"));
        if link.is_symlink() {
            std::fs::remove_file(&link)?;
        }
        return Ok(());
    }
    let path = PathBuf::from(str_of(k, "path"));
    if !path.exists() {
        return Ok(());
    }
    let src = repo_source(k)?;
    let local: Vec<String> = names(&k["local"]).into_iter().filter(|f| inside_path(f) && stays_inside(&path, &path.join(f))).collect();
    if !force {
        let porcelain = git_raw(&path, &read_args(&["status", "--porcelain"]))?;
        let changed = changed_lines(&porcelain, &local).len();
        if changed > 0 {
            fail!("{} has {changed} uncommitted changes. Nothing was removed.", path.display());
        }
    }
    // git refuses a worktree with untracked files: the copied and linked ones go first. A
    // forced remove takes the copies with the worktree, and only the links go first.
    for f in &local {
        let file = path.join(f);
        if file.is_symlink() || (!force && file.is_file()) {
            std::fs::remove_file(&file)?;
        } else if !force && file.is_dir() {
            std::fs::remove_dir_all(&file)?;
        }
    }
    let p = path.to_string_lossy();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&p);
    git(&src, &args)?;
    Ok(())
}

/// Helm's own files at the top of a space folder. Claude Code also writes its local settings
/// into `.claude`.
const OWN_FILES: [&str; 4] = [".herdr", "CLAUDE.md", ".claude", ".DS_Store"];

/// Every file under `dir`, relative to `top`, without .DS_Store.
fn files_under(dir: &Path, top: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() && !p.is_symlink() {
                stack.push(p);
            } else if e.file_name() != ".DS_Store" {
                found.push(p.strip_prefix(top).unwrap_or(&p).to_string_lossy().into_owned());
            }
        }
    }
    found.sort();
    found
}

/// The files of a space folder that are not in a checkout and are not Helm's own: the notes of
/// the agents (.notes/), a skill that an agent made in .claude/skills, and anything else at the
/// top. `complete` refuses while there are any, and `delete` lists them as lost. The paths are
/// relative to the space folder. A folder other than .notes shows as one entry, with a "/".
pub fn extra_files(rec: &Value, space_dir: &Path) -> Vec<String> {
    let checkouts = rec["checkouts"].as_array().cloned().unwrap_or_default();
    let folders: Vec<&str> = checkouts.iter().filter_map(|k| k["folder"].as_str()).collect();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(space_dir).into_iter().flatten().flatten().map(|e| e.path()).collect();
    entries.sort();
    let mut found = Vec::new();
    for p in entries {
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if OWN_FILES.contains(&name.as_str()) || folders.contains(&name.as_str()) {
            continue;
        }
        if name == ".notes" && p.is_dir() && !p.is_symlink() {
            found.extend(files_under(&p, space_dir));
        } else if p.is_dir() && !p.is_symlink() {
            found.push(format!("{name}/"));
        } else {
            found.push(name);
        }
    }
    // The built-in skills are copies of the templates, and a link goes to a checkout's skill.
    let builtin = crate::paths::templates_dir().join("skills");
    let skills = space_dir.join(".claude").join("skills");
    let mut made: Vec<String> = std::fs::read_dir(&skills)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| !e.path().is_symlink() && !builtin.join(e.file_name()).is_dir())
        .map(|e| format!(".claude/skills/{}/", e.file_name().to_string_lossy()))
        .collect();
    made.sort();
    found.extend(made);
    found
}

/// The most rows of extra files that `losses` gives.
const EXTRA_ROWS: usize = 20;

/// What `complete` or `delete` would lose: uncommitted changes, commits not pushed, and the
/// files of the space folder that are not in a checkout (`extra_files`). A row of the last kind
/// has "checkout": "".
pub fn losses(rec: &Value, space_dir: &Path) -> Result<Vec<Value>> {
    let mut found = Vec::new();
    // An external checkout stays where it is, so nothing in it is lost.
    for k in rec["checkouts"].as_array().into_iter().flatten().filter(|k| !k["external"].as_bool().unwrap_or(false)) {
        let folder = str_of(k, "folder");
        let mut s = checkout_status(rec, k)?;
        if s["missing"].as_bool().unwrap_or(false) {
            // The recorded path is gone, for example after a move of the tasks folder. The
            // removal of the space folder still takes its folder there, so check that one.
            let here = space_dir.join(folder);
            if folder.is_empty() || here.symlink_metadata().is_err() {
                continue;
            }
            let mut moved = k.clone();
            moved["path"] = json!(here.to_string_lossy());
            match checkout_status(rec, &moved) {
                Ok(m) if !m["missing"].as_bool().unwrap_or(false) => s = m,
                _ => {
                    found.push(json!({ "checkout": folder, "what": format!("{folder}/ is not a git checkout that Helm can check") }));
                    continue;
                }
            }
        }
        let (dirty, unpushed) = (s["dirty"].as_i64().unwrap_or(0), s["unpushed"].as_i64().unwrap_or(0));
        if dirty > 0 {
            found.push(json!({ "checkout": folder, "what": format!("{dirty} uncommitted changes") }));
        }
        let (branch, head) = (str_of(k, "branch"), s["head"].as_str().unwrap_or(""));
        if unpushed > 0 {
            found.push(json!({ "checkout": folder, "what": format!("{unpushed} commits on {head} not pushed") }));
        }
        // Someone switched the checkout to another branch. `delete` removes the recorded branch,
        // so its commits count too.
        if head != branch && made_branch(k) {
            let path = Path::new(s["path"].as_str().unwrap_or(""));
            let r = format!("refs/heads/{branch}");
            let other =
                git_maybe(path, &["rev-list", "--count", &r, "--not", "--remotes"], false).and_then(|n| n.trim().parse::<i64>().ok());
            if let Some(n) = other.filter(|n| *n > 0) {
                found.push(json!({ "checkout": folder, "what": format!("{n} commits on {branch} not pushed") }));
            }
        }
    }
    let extra = extra_files(rec, space_dir);
    for f in extra.iter().take(EXTRA_ROWS) {
        found.push(json!({ "checkout": "", "what": format!("{f} is not in a checkout") }));
    }
    if extra.len() > EXTRA_ROWS {
        found.push(json!({ "checkout": "", "what": format!("and {} more files that are not in a checkout", extra.len() - EXTRA_ROWS) }));
    }
    Ok(found)
}

/// The branch and sync state of a repo: the rows of `changes`, and of the code host's
/// `github` before it adds the PR.
pub fn repo_state(name: &str, path: &Path) -> Value {
    let maybe = |args: &[&str]| git_maybe(path, args, false).filter(|s| !s.is_empty());
    let branch = maybe(&["rev-parse", "--abbrev-ref", "HEAD"]);
    let dirty = maybe(&read_args(&["status", "--porcelain"])).map_or(0, |s| s.lines().filter(|l| !l.is_empty()).count());
    let upstream = maybe(&["rev-parse", "--abbrev-ref", "@{u}"]);
    let (mut ahead, mut behind) = (0, 0);
    if upstream.is_some() {
        let counts = maybe(&["rev-list", "--left-right", "--count", "HEAD...@{u}"]).unwrap_or_default();
        if let [a, b] = counts.split_whitespace().collect::<Vec<_>>()[..] {
            (ahead, behind) = (a.parse().unwrap_or(0), b.parse().unwrap_or(0));
        }
    } else {
        ahead = maybe(&["rev-list", "--count", "HEAD", "--not", "--remotes"]).and_then(|s| s.parse().ok()).unwrap_or(0);
    }
    let default = maybe(&["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"]).unwrap_or_default();
    let default = default.strip_prefix("origin/").unwrap_or(&default);
    let default_branch = branch.as_deref().is_some_and(|b| b == default || b == "main" || b == "master");
    json!({
        "repo": name, "path": path.to_string_lossy(), "branch": branch, "dirty": dirty,
        "upstream": upstream, "ahead": ahead, "behind": behind, "default_branch": default_branch,
    })
}

/// A changed file of `status --porcelain=v1 -z`: path, status (A M D U R), untracked.
fn parse_status_z(raw: &str) -> BTreeMap<String, Map<String, Value>> {
    let mut files = BTreeMap::new();
    let mut entries = raw.split('\0');
    while let Some(e) = entries.next() {
        if e.len() < 4 {
            continue;
        }
        let (xy, rel) = (e.get(..2).unwrap_or(""), e.get(3..).unwrap_or(""));
        if xy.contains('R') {
            entries.next(); // the old name follows a rename
        }
        // The conflict states first: AA, AU, UA, DD, DU, UD, and UU are not new or deleted files.
        let status = if xy.contains('U') || xy == "AA" || xy == "DD" {
            "U"
        } else if xy == "??" || xy.contains('A') {
            "A"
        } else if xy.contains('D') {
            "D"
        } else if xy.contains('R') {
            "R"
        } else {
            "M"
        };
        let f = json!({ "path": rel, "status": status, "add": 0, "del": 0, "untracked": xy == "??" });
        if let Value::Object(f) = f {
            files.insert(rel.to_string(), f);
        }
    }
    files
}

/// Adds the +/- lines of `diff --numstat -z` to the files. A rename has no path on its own
/// line, so it keeps 0/0.
fn add_numstat(raw: &str, files: &mut BTreeMap<String, Map<String, Value>>) {
    for line in raw.split('\0') {
        let parts: Vec<&str> = line.split('\t').collect();
        if let [add, del, path] = parts[..] {
            if let Some(f) = files.get_mut(path) {
                f.insert("add".into(), json!(add.parse::<u64>().unwrap_or(0)));
                f.insert("del".into(), json!(del.parse::<u64>().unwrap_or(0)));
            }
        }
    }
}

/// The repo's sync state and its changed files: path, status (A M D U R), +/- lines.
pub fn repo_changes(name: &str, path: &Path) -> Value {
    let mut s = repo_state(name, path);
    let raw = git_maybe(path, &read_args(&["status", "--porcelain=v1", "-z", "--untracked-files=all"]), true).unwrap_or_default();
    let mut files = parse_status_z(&raw);
    let has_head = has_ref(path, "HEAD");
    let diff = if has_head { read_args(&["diff", "HEAD", "--numstat", "-z"]) } else { read_args(&["diff", "--numstat", "-z"]) };
    add_numstat(&git_maybe(path, &diff, true).unwrap_or_default(), &mut files);
    // A new file has no diff to count: its lines are all added.
    for f in files.values_mut() {
        if f["status"] == "A" && f["add"] == 0 {
            if let Ok(bytes) = std::fs::read(path.join(f["path"].as_str().unwrap_or(""))) {
                f.insert("add".into(), json!(bytes.iter().filter(|b| **b == b'\n').count()));
            }
        }
    }
    s["files"] = Value::Array(files.into_values().map(Value::Object).collect());
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_files_are_not_changes() {
        let porcelain = " M src/a.rs\n?? .env\n?? node_modules/\n?? node_modules/x/y\n?? notes.md\n";
        let local = vec![".env".to_string(), "node_modules/".to_string()];
        assert_eq!(changed_lines(porcelain, &local), [" M src/a.rs", "?? notes.md"]);
    }

    #[test]
    fn status_z_parsing() {
        let raw = " M src/a.rs\0?? new.txt\0R  b.rs\0old_b.rs\0D  gone.rs\0UU both.rs\0A  added.rs\0AA two.rs\0AU ours.rs\0DD none.rs\0";
        let files = parse_status_z(raw);
        let st: Vec<(&str, &str)> = files.iter().map(|(p, f)| (p.as_str(), f["status"].as_str().unwrap())).collect();
        assert_eq!(
            st,
            [
                ("added.rs", "A"),
                ("b.rs", "R"),
                ("both.rs", "U"),
                ("gone.rs", "D"),
                ("new.txt", "A"),
                ("none.rs", "U"),
                ("ours.rs", "U"),
                ("src/a.rs", "M"),
                ("two.rs", "U")
            ]
        );
        assert_eq!(files["new.txt"]["untracked"], true);
        assert_eq!(files["src/a.rs"]["untracked"], false);
    }

    #[test]
    fn numstat_parsing() {
        let mut files = parse_status_z(" M a.rs\0 M img.png\0R  b.rs\0old.rs\0");
        add_numstat("3\t1\ta.rs\0-\t-\timg.png\x002\t2\t\0old.rs\0b.rs\0", &mut files);
        assert_eq!((files["a.rs"]["add"].clone(), files["a.rs"]["del"].clone()), (json!(3), json!(1)));
        assert_eq!(files["img.png"]["add"], 0);
        assert_eq!(files["b.rs"]["add"], 0);
    }

    #[test]
    fn last_chars_counts_characters() {
        assert_eq!(last_chars("abcdef", 3), "def");
        assert_eq!(last_chars("ab", 3), "ab");
        assert_eq!(last_chars("ééé", 2), "éé");
    }
}
