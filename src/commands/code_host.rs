//! The commands of the code_host connector: github, prs, pr. They hold the flow that every
//! code host shares; the connector does the host's part (connectors/code_host).

use crate::cli::*;
use crate::connectors::code_host::{current, origin_of, par_map, required_with, BranchPr, Checkout, CodeHost, PrTask};
use crate::connectors::has;
use crate::core;
use crate::core::herdr::{herdr, herdr_space_of, refuse_in_terminal, terminals};
use crate::fail;
use crate::util::{git, git_maybe, now_ms, out};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------ github

/// The repo state with its newest PR in "pr" (null on the default branch). A repo on another
/// host gets "not_github": true, a failed lookup "pr_error": the mods read these keys.
fn with_pr(host: &dyn CodeHost, mut s: Value) -> Value {
    s["pr"] = Value::Null;
    let branch = s["branch"].as_str().unwrap_or_default().to_string();
    if s["default_branch"] == true || branch.is_empty() || branch == "HEAD" {
        return s;
    }
    let path = s["path"].as_str().unwrap_or_default().to_string();
    let head = git_maybe(Path::new(&path), &["rev-parse", "HEAD"], false);
    let k = Checkout { path, branch, upstream: s["upstream"].as_str().map(String::from), head };
    match host.branch_pr(&k) {
        BranchPr::None => {}
        BranchPr::Found(pr) => s["pr"] = pr,
        BranchPr::NotHosted => s["not_github"] = json!(true),
        BranchPr::Error(e) => s["pr_error"] = json!(e),
    }
    s
}

fn no_pr(mut s: Value) -> Value {
    s["pr"] = Value::Null;
    s
}

fn github_line(s: &Value) -> String {
    let pr = &s["pr"];
    let what = if pr.is_object() {
        let draft = if pr["isDraft"] == true { " draft" } else { "" };
        let checks = pr["checks"].as_str().unwrap_or("None");
        format!("#{} {}{draft} checks:{checks}", pr["number"], pr["state"].as_str().unwrap_or_default().to_lowercase())
    } else if s["not_github"] == true {
        "not on the code host".to_string()
    } else {
        s["pr_error"].as_str().unwrap_or("no PR").to_string()
    };
    format!("{:<28} {:<30} {what}", s["repo"].as_str().unwrap_or_default(), s["branch"].as_str().unwrap_or_default())
}

/// The repos of a space with their PRs. Without a code host, or without "branch_prs", each repo
/// has "pr": null: the mods still show the repos.
pub fn cmd_github(c: &Ctx, a: &SpaceOrFolder) -> Result<()> {
    let host = current()?.filter(|h| has(&h.capabilities(), "branch_prs"));
    let (space, repos) = core::space_repos(c.space.as_deref(), a.herdr_space.as_deref(), a.folder.as_deref())?;
    let found = par_map(&repos, |(name, path)| {
        let s = core::repo_state(name, path);
        match &host {
            Some(h) => with_pr(h.as_ref(), s),
            None => no_pr(s),
        }
    });
    let text = found.iter().map(github_line).collect::<Vec<_>>().join("\n");
    out(&json!({ "space": space, "at": now_ms(), "repos": found }), if text.is_empty() { "No repos." } else { &text }, c.json);
    Ok(())
}

// ------------------------------------------------------------------ prs

/// The sections of the prs command, in order.
const SECTIONS: [(&str, &str); 7] = [
    ("attention", "Needs your attention"),
    ("approved", "Approved, not in the merge queue"),
    ("queue", "In the merge queue"),
    ("review", "Needs a review"),
    ("mine", "Your other PRs"),
    ("other", "Other open PRs"),
    ("drafts", "Drafts"),
];

/// Sorts the PRs of a section: approved oldest change first, the queue by repo and place,
/// review requests to you first, the rest newest change first.
fn sort_section(key: &str, group: &mut [Value]) {
    let text = |p: &Value, k: &str| p[k].as_str().unwrap_or_default().to_string();
    match key {
        "approved" => group.sort_by_key(|p| text(p, "updatedAt")),
        "queue" => group.sort_by_key(|p| (text(p, "repo"), p["queue"]["position"].as_i64().unwrap_or(0))),
        "review" => group.sort_by_key(|p| (p["requestedMe"] != true, text(p, "createdAt"))),
        _ => group.sort_by_key(|p| std::cmp::Reverse(text(p, "updatedAt"))),
    }
}

/// The PRs in their sections, without empty sections.
fn sections_of(prs: &[Value]) -> Vec<Value> {
    SECTIONS
        .iter()
        .filter_map(|(key, title)| {
            let mut group: Vec<Value> = prs.iter().filter(|p| p["section"] == *key).cloned().collect();
            sort_section(key, &mut group);
            (!group.is_empty()).then(|| json!({ "key": key, "title": title, "prs": group }))
        })
        .collect()
}

/// Links each open PR to the space whose checkout (any repo, any extra checkout) is on its head
/// branch, and gives a linked PR that is no longer open its final state.
fn link_prs(host: &dyn CodeHost, open_prs: &[Value]) -> Result<()> {
    let c = core::config()?;
    let by_key = |key: &str| open_prs.iter().find(|p| p["key"].as_str() == Some(key));
    for (rec, dir) in core::all_spaces(&c)? {
        let mut prs = core::prs_of(&rec);
        let mut changed = false;
        for k in rec["checkouts"].as_array().cloned().unwrap_or_default() {
            let path = Path::new(k["path"].as_str().unwrap_or_default());
            if !path.exists() {
                continue;
            }
            // The repo name on the host, from the remote: the folder name can differ.
            let Some(repo) = host.org_repo(&origin_of(path)) else { continue };
            let p = open_prs
                .iter()
                .find(|p| p["repo"].as_str().map(str::to_lowercase).as_deref() == Some(repo.as_str()) && p["head"] == k["branch"]);
            if let Some(p) = p {
                if !prs.iter().any(|x| x["key"] == p["key"]) {
                    prs.push(json!({
                        "key": p["key"], "repo": p["repo"], "number": p["number"], "title": p["title"], "url": p["url"],
                        "author": p["author"], "kind": "work", "state": "OPEN",
                    }));
                    changed = true;
                }
            }
        }
        for x in prs.iter_mut() {
            let key = x["key"].as_str().unwrap_or_default().to_string();
            let live = by_key(&key);
            if let Some(live) = live {
                if live["title"] != x["title"] {
                    x["title"] = live["title"].clone();
                    changed = true;
                }
            }
            let state = match (live, x["state"].as_str()) {
                (Some(_), _) => Some("OPEN".to_string()),
                (None, Some(s)) if s != "OPEN" => Some(s.to_string()),
                _ => {
                    let number = x["number"].as_u64().unwrap_or(0);
                    host.pr_state(x["repo"].as_str().unwrap_or_default(), number).or_else(|| x["state"].as_str().map(String::from))
                }
            };
            if x["state"].as_str() != state.as_deref() {
                x["state"] = json!(state);
                changed = true;
            }
        }
        if changed {
            // Under the lock, on the record as it is now: another command may have changed it.
            core::update_space(&dir, |r| {
                r["prs"] = Value::Array(prs);
                if let Some(o) = r.as_object_mut() {
                    o.remove("pr");
                }
                Ok(())
            })?;
        }
    }
    Ok(())
}

pub fn cmd_prs(c: &Ctx) -> Result<()> {
    let host = required_with("org_prs")?;
    let found = host.org_prs()?;
    let prs = found["prs"].as_array().cloned().unwrap_or_default();
    link_prs(host.as_ref(), &prs)?;
    let sections = sections_of(&prs);
    let warning = found["warning"].as_str().filter(|w| !w.is_empty());
    let data = json!({
        "at": now_ms(), "viewer": found["viewer"], "host": found["host"], "org": found["org"],
        "refresh_minutes": found["refresh_minutes"], "sections": sections, "rate": found["rate"], "warning": warning,
    });
    let mut text = sections.iter().map(section_text).collect::<Vec<_>>().join("\n");
    if let Some(w) = warning {
        text = format!("{w}\n{text}");
    }
    out(&data, &text, c.json);
    Ok(())
}

fn section_text(s: &Value) -> String {
    let prs = s["prs"].as_array().cloned().unwrap_or_default();
    let mut lines = vec![format!("{} ({})", s["title"].as_str().unwrap_or_default(), prs.len())];
    for p in &prs {
        let title: String = p["title"].as_str().unwrap_or_default().chars().take(60).collect();
        let reasons: Vec<&str> = p["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        lines.push(format!("  {}#{} {title}  {}", p["repo"].as_str().unwrap_or_default(), p["number"], reasons.join(", ")));
    }
    lines.join("\n")
}

// ------------------------------------------------------------------ pr

/// The project and repo folder whose origin is the host's repo `repo`, or None.
fn local_clone(host: &dyn CodeHost, c: &Value, repo: &str) -> Option<(String, String)> {
    let want = repo.to_lowercase();
    let projects = c["projects"].as_object()?;
    projects.iter().find_map(|(name, p)| {
        core::project_repos(p)
            .into_iter()
            .find(|(_, path)| host.org_repo(&origin_of(path)).as_deref() == Some(want.as_str()))
            .map(|(folder, _)| (name.clone(), folder))
    })
}

/// The space with a checkout of `folder` on `branch`.
fn space_on_branch(c: &Value, folder: &str, branch: &str) -> Option<(Value, PathBuf)> {
    core::all_spaces(c)
        .ok()?
        .into_iter()
        .find(|(rec, _)| rec["checkouts"].as_array().into_iter().flatten().any(|k| k["repo"] == folder && k["branch"] == branch))
}

/// The worktree that has `branch` checked out, if any.
fn worktree_on(src: &Path, branch: &str) -> Result<Option<String>> {
    let want = format!("refs/heads/{branch}");
    let mut found = None;
    for block in git(src, &["worktree", "list", "--porcelain"])?.split("\n\n") {
        let field = |name: &str| block.lines().find_map(|l| l.strip_prefix(name).and_then(|r| r.strip_prefix(' ')));
        if field("branch") == Some(want.as_str()) {
            found = field("worktree").map(String::from);
        }
    }
    Ok(found)
}

/// Makes a space for a PR: its record, then a checkout of its branch. The space gets a new
/// folder: a folder that exists is never used or removed. When a step fails, the new folder goes.
fn make_pr_space(c: &Value, a: &PrArgs, info: &Value, project_name: &str, folder: &str) -> Result<(Value, PathBuf)> {
    let root = core::tasks_root(c);
    std::fs::create_dir_all(&root)?;
    let (name, space_dir) = new_space_dir(&root, a.number, &a.repo)?;
    match fill_pr_space(c, a, info, project_name, folder, &name, &space_dir) {
        Ok(rec) => Ok((rec, space_dir)),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&space_dir);
            Err(e)
        }
    }
}

/// The names to try for the space of a PR, in order: pr-N, pr-N-<repo>, pr-N-<repo>-2, ...
fn pr_space_names(number: u64, repo: &str) -> impl Iterator<Item = String> {
    let short: String = repo.to_lowercase().chars().filter(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit()).take(8).collect();
    let with_repo = if short.is_empty() { format!("pr-{number}-repo") } else { format!("pr-{number}-{short}") };
    [format!("pr-{number}"), with_repo.clone()].into_iter().chain((2..).map(move |i| format!("{with_repo}-{i}")))
}

/// Makes the folder of a new PR space with the first free name. `create_dir` fails when the
/// name exists, so this run owns the folder that it makes.
fn new_space_dir(root: &Path, number: u64, repo: &str) -> Result<(String, PathBuf)> {
    for name in pr_space_names(number, repo).take(100) {
        let dir = root.join(&name);
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok((name, dir)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => fail!("Could not make {}: {e}", dir.display()),
        }
    }
    fail!("No free folder name for PR #{number} in {}.", root.display())
}

/// The record, the checkout and the agent files of a new PR space in its new folder.
fn fill_pr_space(c: &Value, a: &PrArgs, info: &Value, project_name: &str, folder: &str, name: &str, space_dir: &Path) -> Result<Value> {
    let head = info["headRefName"].as_str().unwrap_or_default();
    let repo = a.repo.to_lowercase();
    std::fs::create_dir_all(space_dir.join(".notes"))?;
    let title = info["title"].clone();
    let mut rec = json!({
        "name": name, "project": project_name, "goal": format!("PR #{}: {} ({})", a.number, title.as_str().unwrap_or_default(), info["url"].as_str().unwrap_or_default()),
        "created": now_ms() / 1000, "config": crate::paths::config_file().to_string_lossy(), "herdr_space": null, "checkouts": [],
        // The sidebar names the space after the PR and shows its state.
        "title": title,
        "prs": [{ "key": format!("{repo}#{}", a.number), "repo": repo, "number": a.number, "title": info["title"],
                  "url": info["url"], "author": info["author"]["login"], "kind": a.action, "state": "OPEN" }],
    });
    core::save_space(&rec, space_dir)?;
    let p = core::project(c, project_name)?;
    let src = core::project_repos(p).into_iter().find(|(f, _)| f == folder).map(|(_, p)| p).unwrap_or_default();
    if let Err(e) = check_out_pr(c, &mut rec, space_dir, &src, folder, head) {
        fail!("Could not check out {head}: {e}");
    }
    core::write_agent_files(c, &rec, space_dir)?;
    Ok(rec)
}

/// Git allows a branch in one worktree only. When another tool has the PR's branch checked out,
/// the space links that worktree instead of making its own.
fn check_out_pr(c: &Value, rec: &mut Value, space_dir: &Path, src: &Path, folder: &str, head: &str) -> Result<()> {
    let Some(elsewhere) = worktree_on(src, head)? else {
        return core::add_checkout(c, rec, space_dir, folder, None, Some(head), true).map(|_| ());
    };
    let link = space_dir.join(folder);
    std::os::unix::fs::symlink(&elsewhere, &link)?;
    if let Some(list) = rec["checkouts"].as_array_mut() {
        list.push(json!({
            "repo": folder, "label": null, "folder": folder, "branch": head, "base": head,
            "path": elsewhere, "link": link.to_string_lossy(), "local": [], "external": true,
        }));
    }
    core::save_space(rec, space_dir)
}

/// An agent tab with a first message in the space's Herdr space: a new tab when that space is
/// open, else a new Herdr space.
fn open_agent(rec: &mut Value, space_dir: &Path, tab: &str, prompt: &str) -> Result<()> {
    let terms = terminals()?;
    let sid = rec["herdr_space"].as_str().map(String::from);
    let alive = sid.as_ref().is_some_and(|id| terms.keys().any(|addr| herdr_space_of(Some(addr), &terms).as_ref() == Some(id)));
    let argv = crate::connectors::agent::space_argv(rec)?;
    let folder = space_dir.to_string_lossy();
    match sid {
        Some(id) if alive => {
            herdr("tab.new", &json!({ "space": id, "name": tab, "folder": folder, "argv": argv, "task": prompt }))?;
            herdr("space.select", &json!({ "id": id }))?;
        }
        _ => {
            let name = rec["title"].as_str().or(rec["name"].as_str()).unwrap_or_default().to_string();
            let result = herdr("space.new", &json!({ "name": name, "folder": folder, "tab": tab, "argv": argv, "task": prompt }))?;
            rec["herdr_space"] = result["space"].clone();
            let id = rec["herdr_space"].clone();
            core::update_space(space_dir, |r| {
                r["herdr_space"] = id;
                Ok(())
            })?;
        }
    }
    Ok(())
}

/// The code_host capability that a `pr` action needs.
fn pr_capability(action: &str) -> &str {
    match action {
        "enqueue" => "merge_queue",
        other => other,
    }
}

pub fn cmd_pr(c: &Ctx, a: &PrArgs) -> Result<()> {
    let host = required_with(pr_capability(&a.action))?;
    if a.action == "enqueue" {
        refuse_in_terminal("pr enqueue")?;
        let said = host.enqueue(&a.repo, a.number)?;
        out(&json!({ "ok": true }), &said, c.json);
        return Ok(());
    }
    let info = host.pr_info(&a.repo, a.number)?;
    // The branch name of a fork's PR can match a local branch of the repo, such as main. Helm
    // would then check out that branch and later delete it, so it refuses these PRs for now.
    if info["isCrossRepository"].as_bool().unwrap_or(false) {
        fail!("PR #{} of {} comes from a fork. Helm cannot make a space for a fork's PR yet. Check it out yourself.", a.number, a.repo);
    }
    let config = core::config()?;
    let Some((project_name, folder)) = local_clone(host.as_ref(), &config, &a.repo) else {
        fail!("No local clone of {} in any project. Clone it under a project root first.", a.repo);
    };
    let head = info["headRefName"].as_str().unwrap_or_default().to_string();
    let found = space_on_branch(&config, &folder, &head);
    let made = found.is_none();
    let (mut rec, space_dir) = match found {
        Some(f) => f,
        None => make_pr_space(&config, a, &info, &project_name, &folder)?,
    };
    let checkout =
        rec["checkouts"].as_array().into_iter().flatten().find(|k| k["repo"] == folder.as_str() && k["branch"] == head.as_str()).cloned();
    let Some(k) = checkout else { fail!("The space {} has no checkout of {folder} on {head}.", rec["name"]) };
    let notes = space_dir.join(".notes").join(format!("{}-{}.md", a.action, a.number));
    let task = PrTask {
        action: a.action.clone(),
        repo: a.repo.clone(),
        number: a.number,
        title: info["title"].as_str().unwrap_or_default().into(),
        url: info["url"].as_str().unwrap_or_default().into(),
        head: head.clone(),
        base: info["baseRefName"].as_str().unwrap_or_default().into(),
        path: k["path"].as_str().unwrap_or_default().into(),
        notes: notes.to_string_lossy().into(),
        reasons: a.reasons.clone().unwrap_or_else(|| "see the PR".into()),
    };
    let prompt = host.agent_prompt(&task)?;
    open_agent(&mut rec, &space_dir, &format!("{} #{}", a.action, a.number), &prompt)?;
    let name = rec["name"].as_str().unwrap_or_default();
    out(
        &json!({ "space": name, "made": made, "path": space_dir.to_string_lossy(), "notes": notes.to_string_lossy() }),
        &format!("{} space {name}; the {} agent writes to {}", if made { "Made" } else { "Used" }, a.action, notes.display()),
        c.json,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(repo: &str, section: &str, updated: &str) -> Value {
        json!({ "repo": repo, "section": section, "updatedAt": updated, "createdAt": updated, "requestedMe": false, "queue": null })
    }

    #[test]
    fn sections_keep_order_and_drop_empty_ones() {
        let prs = vec![pr("a", "other", "1"), pr("b", "attention", "1"), pr("c", "other", "3")];
        let s = sections_of(&prs);
        assert_eq!(s.iter().map(|x| x["key"].as_str().unwrap()).collect::<Vec<_>>(), ["attention", "other"]);
        assert_eq!(s[1]["prs"][0]["repo"], "c"); // newest change first
    }

    #[test]
    fn section_sort_orders() {
        let mut g = vec![pr("a", "approved", "2"), pr("b", "approved", "1")];
        sort_section("approved", &mut g);
        assert_eq!(g[0]["repo"], "b");

        let mut a = pr("web", "queue", "1");
        a["queue"] = json!({ "position": 2 });
        let mut b = pr("web", "queue", "1");
        b["queue"] = json!({ "position": 1 });
        let mut g = vec![a, b, pr("api", "queue", "1")];
        sort_section("queue", &mut g);
        assert_eq!((g[0]["repo"].as_str(), g[1]["queue"]["position"].as_i64()), (Some("api"), Some(1)));

        let mut asks = pr("x", "review", "5");
        asks["requestedMe"] = json!(true);
        let mut g = vec![pr("y", "review", "1"), asks];
        sort_section("review", &mut g);
        assert_eq!(g[0]["repo"], "x");
    }

    #[test]
    fn repo_state_of_a_new_repo() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "feat"]).unwrap();
        std::fs::write(p.join("a.txt"), "x\n").unwrap();
        let s = core::repo_state("demo", p);
        assert_eq!(s["repo"], "demo");
        assert_eq!(s["dirty"], 1);
        assert_eq!(s["upstream"], Value::Null);
        assert_eq!(s["default_branch"], false);
    }

    #[test]
    fn pr_space_names_in_order() {
        let names: Vec<String> = pr_space_names(12, "Web-App").take(4).collect();
        assert_eq!(names, ["pr-12", "pr-12-webapp", "pr-12-webapp-2", "pr-12-webapp-3"]);
        assert_eq!(pr_space_names(3, "...").nth(1).as_deref(), Some("pr-3-repo"));
    }

    #[test]
    fn new_space_dir_never_takes_a_folder_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("pr-7")).unwrap();
        std::fs::write(root.join("pr-7").join("keep.txt"), "x").unwrap();
        // A file and a dangling link also take a name.
        std::fs::write(root.join("pr-7-web"), "x").unwrap();
        std::os::unix::fs::symlink(root.join("gone"), root.join("pr-7-web-2")).unwrap();
        let (name, made) = new_space_dir(root, 7, "Web").unwrap();
        assert_eq!(name, "pr-7-web-3");
        assert!(made.is_dir());
        assert!(root.join("pr-7").join("keep.txt").is_file());
        let (name, _) = new_space_dir(root, 7, "Web").unwrap();
        assert_eq!(name, "pr-7-web-4");
    }

    #[test]
    fn pr_actions_need_their_capability() {
        assert_eq!(pr_capability("review"), "review");
        assert_eq!(pr_capability("fix"), "fix");
        assert_eq!(pr_capability("enqueue"), "merge_queue");
    }

    #[test]
    fn github_line_texts() {
        let s = json!({ "repo": "web", "branch": "feat", "pr": { "number": 3, "state": "OPEN", "isDraft": true, "checks": "passing" } });
        assert!(github_line(&s).ends_with("#3 open draft checks:passing"));
        let s = json!({ "repo": "web", "branch": "feat", "pr": null, "not_github": true });
        assert!(github_line(&s).ends_with("not on the code host"));
    }
}
