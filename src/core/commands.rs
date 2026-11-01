//! The commands of the core: projects, project, settings, spaces, new, add, status, changes,
//! remove, complete, delete, skills, refresh.

use super::checkout::{make_checkout, project_base, push_checkout, undo_checkout, with_notes};
use super::config::{check_project, find_repo, git_repos, names, project_root, read_raw, tidy_project, update_config, RESERVED};
use super::herdr::{close_herdr_space, herdr, refuse_in_terminal};
use super::*;
use crate::cli::*;
use crate::fail;
use crate::paths::{config_file, templates_dir, DEFAULT_TASKS_ROOT};
use crate::util::{expand, git, git_maybe, has_ref, now_ms, now_secs, out, par_map};
use anyhow::{anyhow, Result};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use unicode_normalization::UnicodeNormalization;

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}

fn list_of(v: &Value) -> Vec<Value> {
    v.as_array().cloned().unwrap_or_default()
}

fn parse_data(data: Option<&str>) -> Result<Map<String, Value>> {
    let change: Value = serde_json::from_str(data.unwrap_or("{}")).map_err(|e| anyhow!("--data is not JSON: {e}"))?;
    match change {
        Value::Object(o) => Ok(o),
        _ => fail!("--data is not a JSON object."),
    }
}

// ---------------------------------------------------------------- projects

pub fn cmd_projects(ctx: &Ctx) -> Result<()> {
    let c = config()?;
    let mut data = Vec::new();
    for (name, p) in c["projects"].as_object().into_iter().flatten() {
        let repos: Vec<String> = project_repos(p).into_iter().map(|(n, _)| n).collect();
        data.push(json!({ "name": name, "root": project_root(p).to_string_lossy(), "repos": repos, "defaults": list_of(&p["defaults"]) }));
    }
    let lines: Vec<String> = data
        .iter()
        .map(|d| {
            let defaults = names(&d["defaults"]);
            let defaults = if defaults.is_empty() { "none".to_string() } else { defaults.join(", ") };
            format!(
                "{}: {} ({} repos, defaults: {defaults})",
                str_of(d, "name"),
                str_of(d, "root"),
                d["repos"].as_array().map_or(0, Vec::len)
            )
        })
        .collect();
    out(&json!(data), &lines.join("\n"), ctx.json);
    Ok(())
}

/// The host part of a git remote URL, lowercase: ssh and https forms give the same.
fn remote_host(url: &str) -> String {
    let url = url.trim().to_lowercase();
    let url = url.strip_suffix(".git").unwrap_or(&url);
    let rest = match url.split_once("://") {
        Some((_, rest)) => rest.rsplit_once('@').map_or(rest, |(_, r)| r).to_string(),
        None => match url.split_once('@') {
            Some((_, rest)) => rest.replacen(':', "/", 1),
            None => url.to_string(),
        },
    };
    rest.split('/').next().unwrap_or("").to_string()
}

/// A repo of a project: its settings, the base new branches start from, the branch it is on,
/// and where it lives (the code host's name for it, else the remote's host).
fn repo_info(name: &str, path: &Path, p: &Value) -> Value {
    let settings = p["repos"].get(name).cloned().unwrap_or_else(|| json!({}));
    let base = match text(&settings, "base") {
        Some(b) => Some(b.to_string()),
        None => project_base(path, p).ok(),
    };
    let remote = git_maybe(path, &["remote", "get-url", "origin"], false).unwrap_or_default();
    let host = crate::connectors::code_host::host_of(&remote).unwrap_or_else(|| {
        let h = remote_host(&remote);
        if h.is_empty() {
            "no remote".to_string()
        } else {
            h
        }
    });
    let branch = git_maybe(path, &["branch", "--show-current"], false).filter(|b| !b.is_empty());
    json!({ "name": name, "settings": settings, "base": base, "branch": branch, "host": host })
}

pub fn cmd_project(ctx: &Ctx, a: &ProjectArgs) -> Result<()> {
    match a.action.as_str() {
        "add" => project_add(a)?,
        "remove" => {
            project(&config()?, &a.name)?;
            update_config(|raw| {
                if let Some(o) = raw["projects"].as_object_mut() {
                    o.shift_remove(&a.name);
                }
                Ok(())
            })?;
            out(&json!({ "removed": a.name }), &format!("Removed project {}. Its spaces stay.", a.name), ctx.json);
            return Ok(());
        }
        "update" => project_update(a)?,
        _ => {}
    }
    project_show(ctx, &config()?, &a.name)
}

fn project_add(a: &ProjectArgs) -> Result<()> {
    if !valid_space_name(&a.name) {
        fail!("A project name is 1-23 of a-z 0-9 -, and starts with a letter or digit.");
    }
    let Some(root) = a.root.as_deref().filter(|r| !r.is_empty()) else { fail!("Give the root folder with --root <path>.") };
    let p = json!({ "root": root, "exclude": [], "defaults": a.default, "repos": {} });
    check_project(&p)?;
    update_config(|raw| {
        if raw["projects"].get(&a.name).is_some() {
            fail!("Project '{}' already exists.", a.name);
        }
        if !raw["projects"].is_object() {
            raw["projects"] = json!({});
        }
        raw["projects"][&a.name] = p;
        Ok(())
    })
}

/// Changes a project under the lock of projects.json, on the project as it is on disk now.
fn project_update(a: &ProjectArgs) -> Result<()> {
    let change = parse_data(a.data.as_deref())?;
    let fields = ["root", "base", "defaults", "exclude", "repos", "claude_md"];
    let mut unknown: Vec<&str> = change.keys().map(String::as_str).filter(|k| !fields.contains(k)).collect();
    if !unknown.is_empty() {
        unknown.sort();
        fail!("Not project fields: {}", unknown.join(", "));
    }
    update_config(|raw| {
        let mut p = project(raw, &a.name)?.clone();
        for (k, v) in change {
            p[k] = v;
        }
        tidy_project(&mut p);
        check_project(&p)?;
        raw["projects"][&a.name] = p;
        Ok(())
    })
}

/// The names of the built-in skills, which every space gets.
fn builtin_skills() -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(templates_dir().join("skills"))
        .map(|e| e.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    found.sort();
    found
}

fn project_show(ctx: &Ctx, c: &Value, name: &str) -> Result<()> {
    let p = project(c, name)?;
    let root = project_root(p);
    let claude_md_path = text(p, "claude_md").map_or_else(|| templates_dir().join("CLAUDE.md"), expand);
    // Each repo needs a few git reads: in parallel, the whole list takes about as long as one repo.
    let repos = par_map(&git_repos(&root), 16, |(n, path)| repo_info(n, path, p));
    let data = json!({
        "name": name, "root": p["root"], "root_path": root.to_string_lossy(),
        "defaults": list_of(&p["defaults"]), "exclude": list_of(&p["exclude"]),
        "base": p.get("base").cloned().unwrap_or(json!("")), "claude_md": p.get("claude_md").cloned().unwrap_or(json!("")),
        "claude_md_path": claude_md_path.to_string_lossy(),
        "skills_builtin": builtin_skills(), "repos": repos,
    });
    let (defaults, exclude) = (names(&data["defaults"]), names(&data["exclude"]));
    let mut lines = vec![format!("{name}: {}", p["root"].as_str().unwrap_or(""))];
    for r in &repos {
        let n = str_of(r, "name");
        let d = if defaults.iter().any(|x| x == n) { " (default)" } else { "" };
        let e = if exclude.iter().any(|x| x == n) { " (excluded)" } else { "" };
        lines.push(format!("  {n}{d}{e}"));
    }
    out(&data, &lines.join("\n"), ctx.json);
    Ok(())
}

// ---------------------------------------------------------------- settings

const SETTING_KEYS: [&str; 8] =
    ["tasks_root", "connectors", "code_host_settings", "agent_settings", "editor_settings", "system_settings", "github", "smart_rename"];

/// Merges `change` into the object at `raw[key]`: a null or "" value removes that key.
fn merge_into(raw: &mut Value, key: &str, change: &Map<String, Value>) {
    if !raw[key].is_object() {
        raw[key] = json!({});
    }
    if let Some(o) = raw[key].as_object_mut() {
        for (k, v) in change {
            if v.is_null() || v == "" {
                o.shift_remove(k);
            } else {
                o.insert(k.clone(), v.clone());
            }
        }
    }
}

/// The legacy "github" block of the settings screen: host, org, refresh_minutes. It goes into
/// code_host_settings.
fn apply_github(raw: &mut Value, g: &Value) -> Result<()> {
    let Some(g) = g.as_object() else { fail!("'github' must be a JSON object.") };
    let mut unknown: Vec<&str> = g.keys().map(String::as_str).filter(|k| !["host", "org", "refresh_minutes"].contains(k)).collect();
    if !unknown.is_empty() {
        unknown.sort();
        fail!("Not GitHub settings: {}", unknown.join(", "));
    }
    if let Some(m) = g.get("refresh_minutes") {
        if !m.as_i64().is_some_and(|m| m >= 1) {
            fail!("The refresh time is a whole number of minutes, 1 or more.");
        }
    }
    // An empty field keeps what is there, as before.
    let kept: Map<String, Value> = g.iter().filter(|(_, v)| !(v.is_null() || *v == "")).map(|(k, v)| (k.clone(), v.clone())).collect();
    merge_into(raw, "code_host_settings", &kept);
    Ok(())
}

/// The legacy "smart_rename" block: {command} goes to agent_settings.smart_command.
fn apply_smart_rename(raw: &mut Value, s: &Value) {
    let command = s["command"].as_str().unwrap_or("").trim().to_string();
    merge_into(raw, "agent_settings", &Map::from_iter([("smart_command".to_string(), json!(command))]));
    if let Some(o) = raw.as_object_mut() {
        o.shift_remove("smart_rename");
    }
}

fn apply_settings(raw: &mut Value, change: &Map<String, Value>) -> Result<()> {
    let mut unknown: Vec<&str> = change.keys().map(String::as_str).filter(|k| !SETTING_KEYS.contains(k)).collect();
    if !unknown.is_empty() {
        unknown.sort();
        fail!("Not settings: {}", unknown.join(", "));
    }
    for (key, value) in change {
        match key.as_str() {
            "tasks_root" => {
                let Some(t) = value.as_str().map(str::trim).filter(|t| !t.is_empty()) else { fail!("Give a tasks folder.") };
                raw["tasks_root"] = json!(t);
            }
            "github" if truthy(value) => apply_github(raw, value)?,
            "github" => {}
            "smart_rename" => apply_smart_rename(raw, value),
            _ => {
                let Some(o) = value.as_object() else { fail!("'{key}' must be a JSON object.") };
                merge_into(raw, key, o);
            }
        }
    }
    Ok(())
}

/// The settings as Helm Settings reads them. The older "github" and "smart_rename" blocks show
/// in their new places. "connectors" is what projects.json picks for each kind. `smart_default`
/// is the Smart rename command of the chosen agent connector, when it has one.
fn settings_view(raw: &Value, smart_default: Option<String>) -> Value {
    let mut g = raw["github"].as_object().cloned().unwrap_or_default();
    g.extend(raw["code_host_settings"].as_object().cloned().unwrap_or_default());
    let or = |key: &str, default: Value| g.get(key).cloned().unwrap_or(default);
    let agent = &raw["agent_settings"];
    let smart = text(agent, "smart_command").or_else(|| text(&raw["smart_rename"], "command"));
    json!({
        "tasks_root": raw.get("tasks_root").cloned().unwrap_or(json!(DEFAULT_TASKS_ROOT)),
        "code_host_settings": { "host": or("host", json!("github.com")), "org": or("org", Value::Null), "refresh_minutes": or("refresh_minutes", json!(5)) },
        "agent_settings": { "smart_command": smart, "smart_default": smart_default, "usage": agent["usage"].as_bool().unwrap_or(false) },
        "connectors": raw.get("connectors").cloned().unwrap_or_else(|| json!({})),
        "config": config_file().to_string_lossy(),
    })
}

pub fn cmd_settings(ctx: &Ctx, a: &SettingsArgs) -> Result<()> {
    let raw = if a.action == "update" {
        let change = parse_data(a.data.as_deref())?;
        update_config(|raw| {
            apply_settings(raw, &change)?;
            Ok(raw.clone())
        })?
    } else {
        read_raw()?
    };
    let smart_default = crate::connectors::agent::current().ok().and_then(|agent| agent.smart_default());
    let mut data = settings_view(&raw, smart_default);
    data["chosen"] = crate::connectors::describe_all();
    out(&data, &settings_text(&data), ctx.json);
    Ok(())
}

/// The settings for a person: one line per setting, then the chosen connectors.
fn settings_text(data: &Value) -> String {
    let (host, agent) = (&data["code_host_settings"], &data["agent_settings"]);
    let or = |v: &Value, none: &str| match v {
        Value::Null => none.to_string(),
        Value::String(s) if s.is_empty() => none.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let smart = match (text(agent, "smart_command"), text(agent, "smart_default")) {
        (Some(cmd), _) => cmd.to_string(),
        (None, Some(default)) => format!("{default} (default)"),
        (None, None) => "the agent's default".to_string(),
    };
    let mut lines = vec![
        format!("Config:          {}", or(&data["config"], "-")),
        format!("Tasks folder:    {}", or(&data["tasks_root"], "-")),
        format!(
            "Code host:       {}, org {}, refresh every {} min",
            or(&host["host"], "-"),
            or(&host["org"], "none"),
            or(&host["refresh_minutes"], "-")
        ),
        format!("Smart rename:    {smart}"),
        format!("Agent usage:     {}", if agent["usage"].as_bool().unwrap_or(false) { "on" } else { "off" }),
    ];
    for (kind, c) in data["chosen"].as_object().into_iter().flatten() {
        let caps = names(&c["capabilities"]);
        let caps = if caps.is_empty() { "nothing".to_string() } else { caps.join(", ") };
        lines.push(format!("{:<17}{} ({caps})", format!("{kind}:"), or(&c["name"], "none")));
    }
    lines.join("\n")
}

// ---------------------------------------------------------------- spaces

pub fn cmd_spaces(ctx: &Ctx) -> Result<()> {
    let c = config()?;
    let mut data = Vec::new();
    for (rec, d) in all_spaces(&c)? {
        let checkouts: Vec<Value> = list_of(&rec["checkouts"]).iter().map(|k| k["folder"].clone()).collect();
        data.push(json!({
            "name": rec["name"], "title": rec.get("title").cloned().unwrap_or(Value::Null),
            "title_source": rec.get("title_source").cloned().unwrap_or(json!("user")),
            "project": rec["project"], "path": d.to_string_lossy(), "prs": prs_of(&rec),
            "herdr_space": rec.get("herdr_space").cloned().unwrap_or(Value::Null), "checkouts": checkouts,
        }));
    }
    let lines: Vec<String> =
        data.iter().map(|d| format!("{} ({}): {}", str_of(d, "name"), str_of(d, "project"), names(&d["checkouts"]).join(", "))).collect();
    out(&json!(data), &lines.join("\n"), ctx.json);
    Ok(())
}

/// A space name from a short line of text: lowercase words joined by dashes, cut at a word to
/// fit a space name (23 characters), never a usual branch name. mods/helm-task has the same.
pub fn slugify(text: &str) -> String {
    let plain: String = text.nfkd().filter(char::is_ascii).collect::<String>().to_lowercase();
    let words: Vec<&str> = plain.split(|ch: char| !ch.is_ascii_lowercase() && !ch.is_ascii_digit()).filter(|w| !w.is_empty()).collect();
    let mut name = String::new();
    for w in &words {
        if name.len() + w.len() + usize::from(!name.is_empty()) > 23 {
            break;
        }
        if !name.is_empty() {
            name.push('-');
        }
        name.push_str(w);
    }
    if name.is_empty() {
        name = words.first().map_or_else(|| "space".to_string(), |w| w[..w.len().min(23)].to_string());
    }
    if RESERVED.contains(&name.as_str()) {
        return format!("{}-work", &name[..name.len().min(18)]);
    }
    name
}

/// `base`, or base-2, base-3 ... up to base-99: the first name that is not taken.
fn first_free(base: &str, taken: impl Fn(&str) -> bool) -> Result<String> {
    if !taken(base) {
        return Ok(base.to_string());
    }
    for i in 2..100 {
        let keep = 23usize.saturating_sub(i.to_string().len() + 1);
        let n = format!("{}-{i}", &base[..base.len().min(keep)]);
        if !taken(&n) {
            return Ok(n);
        }
    }
    fail!("No free name for {base}.")
}

/// A name no space has and no repo has a branch of.
fn free_name(c: &Value, base: &str, repos: &[PathBuf]) -> Result<String> {
    let root = tasks_root(c);
    first_free(base, |n| root.join(n).exists() || repos.iter().any(|r| has_ref(r, &format!("refs/heads/{n}"))))
}

/// Fails when a repo has the branch checked out in a worktree: git allows a branch in one only.
fn check_branch_free(repo: &str, src: &Path, branch: &str) -> Result<()> {
    for block in git(src, &["worktree", "list", "--porcelain"])?.split("\n\n") {
        let field = |name: &str| block.lines().find_map(|l| l.strip_prefix(name).and_then(|r| r.strip_prefix(' ')));
        if field("branch") == Some(&format!("refs/heads/{branch}")) {
            fail!("The branch {branch} of {repo} is checked out in {}. Pick another name.", field("worktree").unwrap_or("?"));
        }
    }
    Ok(())
}

pub fn cmd_new(ctx: &Ctx, a: &NewArgs) -> Result<()> {
    let c = config()?;
    let p = project(&c, &a.project)?;
    let repos: Vec<String> = if a.repo.is_empty() { names(&p["defaults"]) } else { a.repo.clone() };
    let known = project_repos(p);
    let unknown: Vec<&str> = repos.iter().filter(|r| find_repo(&known, r).is_none()).map(String::as_str).collect();
    if !unknown.is_empty() {
        fail!("Not repos of project {}: {}", a.project, unknown.join(", "));
    }
    let title = a.title.as_deref().filter(|t| !t.is_empty());
    let sources: Vec<PathBuf> = repos.iter().filter_map(|r| find_repo(&known, r).cloned()).collect();
    // A short line of text gives the title, and the name and branch made from it.
    let name = match (a.name.as_deref().filter(|n| !n.is_empty()), title) {
        (Some(n), _) => n.to_string(),
        (None, Some(t)) => free_name(&c, &slugify(t), &sources)?,
        (None, None) => fail!("Say what the space is for: herdr-helm new --title 'SSO login times out' (or give a name)."),
    };
    // The record and CLAUDE.md keep the title as the goal when none is given, but the agent gets
    // a first message only when one was written.
    let message = a.goal.as_deref().filter(|g| !g.is_empty());
    let goal = message.or(title);
    if !valid_space_name(&name) || RESERVED.contains(&name.as_str()) {
        fail!("A space name is 1-23 of a-z 0-9 -, starts with a letter or digit, and is not a usual branch name.");
    }
    let space_dir = tasks_root(&c).join(&name);
    if space_dir.exists() {
        fail!("A space named {name} exists already. Pick another name.");
    }
    // Everything is checked before anything is made.
    for (r, src) in repos.iter().zip(&sources) {
        check_branch_free(r, src, &name)?;
    }
    // create_dir, not create_dir_all: when two runs start with the same name, only one gets the
    // folder. The other stops here, before it can remove the folder and the first run's work.
    std::fs::create_dir_all(tasks_root(&c))?;
    match std::fs::create_dir(&space_dir) {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => fail!("A space named {name} exists already. Pick another name."),
        r => r?,
    }
    std::fs::create_dir(space_dir.join(".notes"))?;
    let rec = json!({
        "name": name, "title": title.unwrap_or(&name), "project": a.project, "goal": goal, "created": now_secs(),
        "config": config_file().to_string_lossy(), "herdr_space": null, "checkouts": [], "prs": [],
    });
    // All or nothing: when a later step fails, the checkouts, their branches and the folder go.
    // make_checkouts removes its own checkouts when one of them fails.
    let (mut rec, added) =
        match save_space(&rec, &space_dir).and_then(|_| make_checkouts(&c, rec, &space_dir, &repos, &sources, a.progress)) {
            Ok(made) => made,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&space_dir);
                return Err(e);
            }
        };
    let finished =
        write_agent_files(&c, &rec, &space_dir)
            .and_then(|_| if a.open { open_space(&mut rec, &space_dir, &a.agent, message) } else { Ok(()) });
    if let Err(e) = finished {
        undo_space(&rec, &space_dir);
        return Err(e.context("Could not make the space"));
    }
    let mut lines = vec![format!("Space {name} at {}", space_dir.display())];
    lines.extend(added.iter().map(|k| format!("  {}: {} from {}", str_of(k, "folder"), str_of(k, "branch"), str_of(k, "base"))));
    lines.extend(added.iter().flat_map(|k| names(&k["notes"])).map(|n| format!("  note: {n}")));
    let data = json!({ "space": name, "path": space_dir.to_string_lossy(), "checkouts": added, "error": null });
    out(&data, &lines.join("\n"), ctx.json);
    Ok(())
}

/// Makes a checkout of each repo, all at once. All or nothing: when one fails, it removes what
/// was made, so a second try with the same name works.
fn make_checkouts(
    c: &Value,
    rec: Value,
    space_dir: &Path,
    repos: &[String],
    sources: &[PathBuf],
    progress: bool,
) -> Result<(Value, Vec<Value>)> {
    let say = |text: String| {
        if progress {
            eprintln!("{text}");
        }
    };
    say(format!("Fetching {}", repos.join(", ")));
    par_map(sources, 8, |src| git_maybe(src, &["fetch", "--quiet", "origin"], false));
    say(format!("Checking out {}", repos.join(", ")));
    let snapshot = rec.clone();
    let shared = Mutex::new((rec, 0usize));
    let results = par_map(repos, 8, |r| -> Result<Value, String> {
        let (k, notes) = make_checkout(c, &snapshot, space_dir, r, None, None, false).map_err(|e| format!("{r}: {e:#}"))?;
        // One writer of the record at a time.
        let mut guard = shared.lock().map_err(|_| format!("{r}: the record lock is broken"))?;
        let (rec, done) = &mut *guard;
        push_checkout(rec, &k);
        save_space(rec, space_dir).map_err(|e| format!("{r}: {e:#}"))?;
        *done += 1;
        say(format!("{done}/{} checked out ({r})", repos.len()));
        Ok(with_notes(k, notes))
    });
    let (rec, _) = shared.into_inner().unwrap_or_else(|e| e.into_inner());
    let failed = results.iter().find_map(|r| r.as_ref().err().cloned());
    if let Some(failed) = failed {
        undo_space(&rec, space_dir);
        fail!("Could not make the space: {failed}");
    }
    Ok((rec, results.into_iter().flatten().collect()))
}

/// Removes the checkouts, their new branches and the folder of a space that failed to make.
fn undo_space(rec: &Value, space_dir: &Path) {
    for k in list_of(&rec["checkouts"]) {
        undo_checkout(&k);
    }
    let _ = std::fs::remove_dir_all(space_dir);
}

/// Opens the space in Herdr. The first tab runs the agent, with the goal as its first message
/// when one was written, or a plain shell.
fn open_space(rec: &mut Value, space_dir: &Path, agent: &str, message: Option<&str>) -> Result<()> {
    let mut args = json!({ "name": rec["title"], "folder": space_dir.to_string_lossy(), "tab": "main" });
    if agent == "agent" || agent == "claude" {
        args["argv"] = json!(crate::connectors::agent::current()?.start_argv()?);
        if let Some(m) = message {
            args["task"] = json!(m);
        }
    }
    let result = herdr("space.new", &args)?;
    let id = result.get("space").cloned().unwrap_or(Value::Null);
    *rec = update_space(space_dir, |fresh| {
        fresh["herdr_space"] = id;
        Ok(())
    })?
    .0;
    Ok(())
}

pub fn cmd_add(ctx: &Ctx, a: &AddArgs) -> Result<()> {
    let c = config()?;
    let (mut rec, space_dir) = load_space(&c, ctx.space.as_deref())?;
    let k = add_checkout(&c, &mut rec, &space_dir, &a.repo, a.label.as_deref(), None, true)?;
    write_agent_files(&c, &rec, &space_dir)?;
    let mut lines = vec![format!("Added {} on {} from {}", str_of(&k, "path"), str_of(&k, "branch"), str_of(&k, "base"))];
    lines.extend(names(&k["notes"]).into_iter().map(|n| format!("  note: {n}")));
    out(&k, &lines.join("\n"), ctx.json);
    Ok(())
}

fn status_line(s: &Value) -> String {
    let folder = str_of(s, "folder");
    if s["missing"].as_bool().unwrap_or(false) {
        return format!("{folder:<28} missing");
    }
    let dirty = s["dirty"].as_i64().unwrap_or(0);
    let mut bits = vec![str_of(s, "head").to_string(), if dirty > 0 { format!("{dirty} changed") } else { "clean".into() }];
    bits.push(match s["upstream"].as_str() {
        Some(u) => format!("+{}/-{} {u}", s["ahead"], s["behind"]),
        None => format!("{} not pushed", s["unpushed"]),
    });
    if let Some(m) = s["merged"].as_bool() {
        bits.push(if m { "merged".into() } else { "not merged".into() });
    }
    format!("{folder:<28} {}", bits.join(" · "))
}

pub fn cmd_status(ctx: &Ctx) -> Result<()> {
    let c = config()?;
    let (rec, space_dir) = load_space(&c, ctx.space.as_deref())?;
    let checkouts = list_of(&rec["checkouts"]).iter().map(|k| checkout_status(&rec, k)).collect::<Result<Vec<_>>>()?;
    let mut lines = vec![format!("Space {} ({})", str_of(&rec, "name"), str_of(&rec, "project"))];
    lines.extend(checkouts.iter().map(status_line));
    let data = json!({ "space": rec["name"], "project": rec["project"], "path": space_dir.to_string_lossy(), "checkouts": checkouts });
    out(&data, &lines.join("\n"), ctx.json);
    Ok(())
}

pub fn cmd_changes(ctx: &Ctx, a: &SpaceOrFolder) -> Result<()> {
    let (space, repos) = space_repos(ctx.space.as_deref(), a.herdr_space.as_deref(), a.folder.as_deref())?;
    let found = par_map(&repos, 8, |(n, p)| repo_changes(n, p));
    let lines: Vec<String> = found
        .iter()
        .map(|r| {
            let files = r["files"].as_array().map_or(0, Vec::len);
            format!("{:<28} {:<24} {files} changed, +{}/-{}", str_of(r, "repo"), str_of(r, "branch"), r["ahead"], r["behind"])
        })
        .collect();
    let text = if lines.is_empty() { "No repos.".to_string() } else { lines.join("\n") };
    out(&json!({ "space": space, "at": now_ms(), "repos": found }), &text, ctx.json);
    Ok(())
}

pub fn cmd_remove(ctx: &Ctx, a: &RemoveArgs) -> Result<()> {
    let c = config()?;
    let (rec, space_dir) = load_space(&c, ctx.space.as_deref())?;
    let Some(k) = list_of(&rec["checkouts"]).into_iter().find(|k| k["folder"] == a.checkout.as_str()) else {
        fail!("No checkout '{}' in this space.", a.checkout)
    };
    if text(&k, "label").is_none() {
        fail!("The main checkout of a repo goes only with the whole space (Complete or Delete in Herdr).");
    }
    let s = checkout_status(&rec, &k)?;
    if !s["missing"].as_bool().unwrap_or(false) {
        let dirty = s["dirty"].as_i64().unwrap_or(0);
        if dirty > 0 {
            fail!("{} has {dirty} uncommitted changes. Commit and merge them first.", str_of(&k, "folder"));
        }
        if !s["merged"].as_bool().unwrap_or(false) {
            fail!("{} is not merged into the space branch. Merge it first.", str_of(&k, "branch"));
        }
        let src = repo_source(&k)?;
        remove_checkout_dir(&k, false)?;
        // Merged into the space branch, checked above; -d would check against the root's HEAD.
        if made_branch(&k) {
            git(&src, &["branch", "-D", str_of(&k, "branch")])?;
        }
    }
    let (rec, ()) = update_space(&space_dir, |fresh| {
        if let Some(list) = fresh["checkouts"].as_array_mut() {
            list.retain(|x| x["folder"] != k["folder"]);
        }
        Ok(())
    })?;
    write_agent_files(&c, &rec, &space_dir)?;
    let (folder, branch) = (str_of(&k, "folder"), str_of(&k, "branch"));
    let text = if made_branch(&k) {
        format!("Removed {folder} and its branch {branch}")
    } else {
        format!("Removed {folder}. The branch {branch} was there before Helm, so it stays.")
    };
    out(&json!({ "removed": folder }), &text, ctx.json);
    Ok(())
}

/// Removes the worktree of every checkout that is there, then the space folder, then prunes the
/// worktree list of each repo. `drop_branches` also deletes each local branch that Helm made.
fn remove_space(rec: &Value, space_dir: &Path, force: bool, drop_branches: bool) -> Result<()> {
    let mut sources = BTreeSet::new();
    for k in list_of(&rec["checkouts"]) {
        if !Path::new(str_of(&k, "path")).exists() {
            continue;
        }
        let src = repo_source(&k)?;
        remove_checkout_dir(&k, force)?;
        if drop_branches && made_branch(&k) {
            git_maybe(&src, &["branch", "-D", str_of(&k, "branch")], false);
        }
        sources.insert(src);
    }
    std::fs::remove_dir_all(space_dir)?;
    for src in sources {
        git_maybe(&src, &["worktree", "prune"], false);
    }
    Ok(())
}

pub fn cmd_complete(ctx: &Ctx, a: &CompleteArgs) -> Result<()> {
    refuse_in_terminal("complete")?;
    complete_space(ctx, a)
}

/// Ends the Herdr space after the removal. The space is gone by then, so a failure here is a
/// warning, not an error.
fn close_after(rec: &Value, space_dir: &Path) {
    if let Err(e) = close_herdr_space(rec, space_dir) {
        eprintln!("herdr-helm: the space is removed, but Herdr did not close it: {e:#}");
    }
}

/// `complete` without the terminal check: the tests call it.
pub(super) fn complete_space(ctx: &Ctx, a: &CompleteArgs) -> Result<()> {
    let c = config()?;
    let (rec, space_dir) = load_space(&c, ctx.space.as_deref())?;
    let problems = losses(&rec, &space_dir)?;
    if a.check || !problems.is_empty() {
        let text = if problems.is_empty() {
            "Ready to complete.".to_string()
        } else {
            let rows: Vec<String> = problems.iter().map(|p| format!("  {}", loss_line(p))).collect();
            format!("Not ready:\n{}", rows.join("\n"))
        };
        out(&json!({ "ok": problems.is_empty(), "problems": problems }), &text, ctx.json);
        if !problems.is_empty() {
            let _ = std::io::stdout().flush();
            std::process::exit(1);
        }
        return Ok(());
    }
    remove_space(&rec, &space_dir, false, false)?;
    if a.close {
        close_after(&rec, &space_dir);
    }
    let checkouts = list_of(&rec["checkouts"]);
    let kept: Vec<&str> = checkouts.iter().map(|k| str_of(k, "branch")).collect();
    let shown: Vec<String> = checkouts.iter().map(|k| format!("{} {}", str_of(k, "repo"), str_of(k, "branch"))).collect();
    out(
        &json!({ "ok": true, "kept_branches": kept }),
        &format!("Completed {}. Branches kept: {}", str_of(&rec, "name"), shown.join(", ")),
        ctx.json,
    );
    Ok(())
}

/// A row of `losses` for a person: "<checkout>: <what>", or only "<what>" for a file that is
/// not in a checkout.
fn loss_line(p: &Value) -> String {
    match str_of(p, "checkout") {
        "" => str_of(p, "what").to_string(),
        k => format!("{k}: {}", str_of(p, "what")),
    }
}

pub fn cmd_delete(ctx: &Ctx, a: &DeleteArgs) -> Result<()> {
    refuse_in_terminal("delete")?;
    delete_space(ctx, a)
}

/// `delete` without the terminal check: the tests call it. Both forms print
/// {"space", "deleted", "losses"}. Without --yes, "deleted" is false and nothing changes.
pub(super) fn delete_space(ctx: &Ctx, a: &DeleteArgs) -> Result<()> {
    let c = config()?;
    let (rec, space_dir) = load_space(&c, ctx.space.as_deref())?;
    let lost = losses(&rec, &space_dir)?;
    let name = str_of(&rec, "name");
    if !a.yes {
        let rows: Vec<String> = lost.iter().map(|p| format!("  lost: {}", loss_line(p))).collect();
        let rows = if rows.is_empty() { "  Nothing unsaved is lost.".to_string() } else { rows.join("\n") };
        let text = format!(
            "Delete removes every checkout of this space, and the local branches that Helm made.\n{rows}\nRun again with --yes to delete."
        );
        out(&json!({ "space": name, "deleted": false, "losses": lost }), &text, ctx.json);
        return Ok(());
    }
    remove_space(&rec, &space_dir, true, true)?;
    if a.close {
        close_after(&rec, &space_dir);
    }
    out(&json!({ "space": name, "deleted": true, "losses": lost }), &format!("Deleted {name}."), ctx.json);
    Ok(())
}

// ---------------------------------------------------------------- skills and refresh

fn report_by_space(ctx: &Ctx, done: Map<String, Value>, none: &str) {
    let lines: Vec<String> = done.iter().map(|(n, v)| format!("{n}: {}", names(v).join(", "))).collect();
    let text = if lines.is_empty() { none.to_string() } else { lines.join("\n") };
    out(&Value::Object(done), &text, ctx.json);
}

/// Links the skills of every space both ways again, for example for spaces made before.
pub fn cmd_skills(ctx: &Ctx, _a: &SkillsArgs) -> Result<()> {
    let c = config()?;
    let mut done = Map::new();
    for (rec, d) in all_spaces(&c)? {
        let linked = link_skills(&rec, &d)?;
        if !linked.is_empty() {
            done.insert(str_of(&rec, "name").to_string(), json!(linked));
        }
    }
    report_by_space(ctx, done, "Every space has all its skills.");
    Ok(())
}

/// Writes CLAUDE.md and the built-in skills of every space again, after an update. A space that
/// fails does not stop the others: its row says "error: <why>".
pub fn cmd_refresh(ctx: &Ctx) -> Result<()> {
    let c = config()?;
    let mut done = Map::new();
    for (rec, d) in all_spaces(&c)? {
        let changed = write_agent_files(&c, &rec, &d).unwrap_or_else(|e| vec![format!("error: {e:#}")]);
        if !changed.is_empty() {
            done.insert(str_of(&rec, "name").to_string(), json!(changed));
        }
    }
    report_by_space(ctx, done, "Every space is up to date.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_rules() {
        assert_eq!(slugify("SSO login times out"), "sso-login-times-out");
        assert_eq!(slugify("Café déjà vu!"), "cafe-deja-vu");
        assert_eq!(slugify("one two three four five six seven"), "one-two-three-four-five");
        assert_eq!(slugify("abcdefghijklmnopqrstuvwxyz0123"), "abcdefghijklmnopqrstuvw");
        assert_eq!(slugify("!!!"), "space");
        assert_eq!(slugify(""), "space");
        assert_eq!(slugify("Main"), "main-work");
        assert_eq!(slugify("HEAD"), "head-work");
        assert_eq!(slugify("fix #123: crash"), "fix-123-crash");
    }

    #[test]
    fn first_free_counts_up() {
        assert_eq!(first_free("x", |_| false).unwrap(), "x");
        assert_eq!(first_free("x", |n| n == "x").unwrap(), "x-2");
        assert_eq!(first_free("x", |n| n == "x" || n == "x-2").unwrap(), "x-3");
        let long = "a".repeat(23);
        assert_eq!(first_free(&long, |n| n == long).unwrap(), format!("{}-2", "a".repeat(21)));
        let n = first_free(&long, |n| !n.ends_with("-10")).unwrap();
        assert_eq!(n, format!("{}-10", "a".repeat(20)));
        assert!(first_free("x", |_| true).is_err());
    }

    #[test]
    fn remote_hosts() {
        assert_eq!(remote_host("git@github.com:org/repo.git"), "github.com");
        assert_eq!(remote_host("https://user@code.example.org/org/p/_git/repo"), "code.example.org");
        assert_eq!(remote_host("ssh://git@GitLab.example.com:22/x/y"), "gitlab.example.com:22");
        assert_eq!(remote_host(""), "");
    }

    #[test]
    fn settings_update_and_view() {
        let mut raw =
            json!({ "tasks_root": "~/t", "github": { "host": "ghe.example.com", "org": "acme" }, "smart_rename": { "command": "old" } });
        let view = settings_view(&raw, None);
        assert_eq!(view["code_host_settings"], json!({ "host": "ghe.example.com", "org": "acme", "refresh_minutes": 5 }));
        assert_eq!(view["agent_settings"]["smart_default"], Value::Null);
        assert_eq!(view["agent_settings"]["smart_command"], "old");

        let change = parse_data(Some(r#"{"github": {"org": "", "refresh_minutes": 10}, "smart_rename": {"command": " new "}}"#)).unwrap();
        apply_settings(&mut raw, &change).unwrap();
        assert_eq!(raw["code_host_settings"], json!({ "refresh_minutes": 10 }));
        assert_eq!(raw["agent_settings"], json!({ "smart_command": "new" }));
        assert!(raw.get("smart_rename").is_none());
        let view = settings_view(&raw, Some("agent -p".into()));
        assert_eq!(view["code_host_settings"], json!({ "host": "ghe.example.com", "org": "acme", "refresh_minutes": 10 }));
        assert_eq!(view["agent_settings"], json!({ "smart_command": "new", "smart_default": "agent -p", "usage": false }));

        apply_settings(&mut raw, &parse_data(Some(r#"{"agent_settings": {"smart_command": "", "usage": true}}"#)).unwrap()).unwrap();
        assert_eq!(
            settings_view(&raw, Some("agent -p".into()))["agent_settings"],
            json!({ "smart_command": null, "smart_default": "agent -p", "usage": true })
        );
        apply_settings(&mut raw, &parse_data(Some(r#"{"connectors": {"code_host": "github"}}"#)).unwrap()).unwrap();
        assert_eq!(raw["connectors"], json!({ "code_host": "github" }));

        let err = |data: &str| apply_settings(&mut raw.clone(), &parse_data(Some(data)).unwrap()).unwrap_err().to_string();
        assert!(err(r#"{"colour": 1}"#).contains("Not settings: colour"));
        assert!(err(r#"{"tasks_root": " "}"#).contains("tasks folder"));
        assert!(err(r#"{"github": {"refresh_minutes": 0}}"#).contains("refresh time"));
        assert!(err(r#"{"github": {"user": "x"}}"#).contains("Not GitHub settings: user"));
        assert!(err(r#"{"agent_settings": 3}"#).contains("must be a JSON object"));
        assert!(parse_data(Some("[1]")).is_err());
        assert!(parse_data(Some("{")).is_err());
    }

    #[test]
    fn settings_text_is_not_json() {
        let mut data = settings_view(&json!({ "tasks_root": "~/t", "code_host_settings": { "org": "acme" } }), Some("agent -p".into()));
        data["chosen"] =
            json!({ "code_host": { "name": "github", "capabilities": ["prs"] }, "agent": { "name": "none", "capabilities": [] } });
        let text = settings_text(&data);
        assert!(!text.trim_start().starts_with('{'), "{text}");
        assert!(text.contains("Tasks folder:    ~/t"), "{text}");
        assert!(text.contains("github.com, org acme, refresh every 5 min"), "{text}");
        assert!(text.contains("Smart rename:    agent -p (default)"), "{text}");
        assert!(text.contains("code_host:       github (prs)") && text.contains("agent:           none (nothing)"), "{text}");
    }

    #[test]
    fn loss_lines() {
        assert_eq!(loss_line(&json!({ "checkout": "web", "what": "2 uncommitted changes" })), "web: 2 uncommitted changes");
        assert_eq!(loss_line(&json!({ "checkout": "", "what": ".notes/a.md is not in a checkout" })), ".notes/a.md is not in a checkout");
    }

    #[test]
    fn status_lines() {
        let s = json!({ "folder": "web", "head": "s1", "dirty": 2, "upstream": "origin/s1", "ahead": 1, "behind": 0, "unpushed": 1 });
        assert_eq!(status_line(&s), format!("{:<28} s1 · 2 changed · +1/-0 origin/s1", "web"));
        let s = json!({ "folder": "web.api", "head": "s1-api", "dirty": 0, "upstream": null, "ahead": 0, "behind": 0, "unpushed": 3, "merged": false });
        assert_eq!(status_line(&s), format!("{:<28} s1-api · clean · 3 not pushed · not merged", "web.api"));
        assert_eq!(status_line(&json!({ "folder": "x", "missing": true })), format!("{:<28} missing", "x"));
    }
}
