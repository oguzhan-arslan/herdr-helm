//! The code_host kind: where the repos live, their PRs, reviews, and merge queue.
//! Built in: github (github.com and GitHub Enterprise, through gh). Others: an external program
//! through the command connector. docs/connectors-code-host.md describes every method.

mod github;

use crate::util::git_maybe;
use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::Path;

/// A checkout to find the PR of.
#[derive(Serialize)]
pub struct Checkout {
    pub path: String,
    pub branch: String,
    pub upstream: Option<String>,
    /// The HEAD commit.
    pub head: Option<String>,
}

/// The PR of a checkout.
pub enum BranchPr {
    None,
    Found(Value),
    /// The repo is on a host that this connector does not serve.
    NotHosted,
    Error(String),
}

/// What an agent needs to review or fix a PR.
#[derive(Serialize)]
pub struct PrTask {
    pub action: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub head: String,
    pub base: String,
    pub path: String,
    pub notes: String,
    pub reasons: String,
}

/// A code host. Every method is also an op of the command form, with the same name.
pub trait CodeHost: Send + Sync {
    /// The connector's name, e.g. "github".
    fn name(&self) -> String;
    /// What it can do: "branch_prs", "org_prs", "review", "fix", "merge_queue".
    fn capabilities(&self) -> Vec<String>;
    /// The host name of a git remote URL as the connector names it, or None for a URL it does
    /// not serve.
    fn host_of(&self, remote_url: &str) -> Option<String>;
    /// The repo name (lowercase) when the remote is a repo of the configured org, else None.
    fn org_repo(&self, remote_url: &str) -> Option<String>;
    /// The newest PR of the checkout's branch, in any state.
    fn branch_pr(&self, checkout: &Checkout) -> BranchPr;
    /// The org's open PRs, each with its "section":
    /// {"viewer", "host", "org", "refresh_minutes", "prs": [...], "rate": {...}}.
    fn org_prs(&self) -> Result<Value>;
    /// One PR of an org repo: {number, title, url, headRefName, baseRefName, author,
    /// isCrossRepository}. isCrossRepository is true for a PR from a fork.
    fn pr_info(&self, repo: &str, number: u64) -> Result<Value>;
    /// The state of a PR of an org repo (OPEN, MERGED, CLOSED), or None when unknown.
    fn pr_state(&self, repo: &str, number: u64) -> Option<String>;
    /// The PRs of a branch of an org repo, in any state:
    /// [{number, title, url, state, author, headRefName}].
    fn branch_prs(&self, repo: &str, branch: &str) -> Vec<Value>;
    /// The repo (lowercase) and number of a PR web URL of this host, or None.
    fn pr_ref(&self, url: &str) -> Option<(String, u64)>;
    /// The PR at a web URL, as branch_prs gives it plus "repo", or None.
    fn pr_at(&self, url: &str) -> Option<Value>;
    /// Merges the PR when it is ready, through the merge queue where the repo has one. Never
    /// merges at once. Returns a message for the person.
    fn enqueue(&self, repo: &str, number: u64) -> Result<String>;
    /// The first message of a review or fix agent.
    fn agent_prompt(&self, task: &PrTask) -> Result<String>;
}

/// The chosen code host: projects.json "connectors.code_host" (default "github"). None when
/// the kind is off ("none").
pub fn current() -> Result<Option<Box<dyn CodeHost>>> {
    match super::choice("code_host", "github") {
        super::Choice::Command(program) => Ok(Some(Box::new(super::command::Command::new("code_host", program)))),
        super::Choice::Builtin(name) => match name.as_str() {
            "github" => Ok(Some(Box::new(github::Github::new()))),
            "none" => Ok(None),
            other => Err(anyhow!("unknown code_host connector '{other}' (built in: github, none)")),
        },
    }
}

/// The chosen code host, or an error that says the kind is off.
pub fn required() -> Result<Box<dyn CodeHost>> {
    current()?.ok_or_else(|| anyhow!("No code host: projects.json has \"connectors\": {{\"code_host\": \"none\"}}."))
}

/// The chosen code host, when it can do `capability`; else an error that says what is missing.
pub fn required_with(capability: &str) -> Result<Box<dyn CodeHost>> {
    let host = required()?;
    if !super::has(&host.capabilities(), capability) {
        return Err(anyhow!("The code_host connector '{}' cannot do '{capability}'.", host.name()));
    }
    Ok(host)
}

pub fn describe() -> Value {
    match current() {
        Ok(Some(h)) => json!({ "name": h.name(), "capabilities": h.capabilities() }),
        _ => super::none(),
    }
}

/// The code host a git remote URL belongs to, as the connector names it (e.g. "github"), or
/// None when the chosen connector does not know the URL. The core uses it for the project page.
pub fn host_of(remote_url: &str) -> Option<String> {
    current().ok().flatten()?.host_of(remote_url)
}

/// host/org/repo of a git remote URL, lowercase, so the ssh and https forms match.
pub fn remote_key(url: &str) -> String {
    let url = url.trim().to_lowercase();
    let url = url.strip_suffix(".git").unwrap_or(&url);
    if let Some((_, rest)) = url.split_once("://") {
        return rest.split_once('@').map_or(rest, |(_, r)| r).to_string();
    }
    match url.split_once('@') {
        Some((_, rest)) => rest.replacen(':', "/", 1),
        None => url.to_string(),
    }
}

/// The origin URL of a checkout, or "".
pub fn origin_of(path: &Path) -> String {
    git_maybe(path, &["remote", "get-url", "origin"], false).unwrap_or_default()
}

/// `f` of each item, on up to 8 threads, in the order of the items.
pub fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    crate::util::par_map(items, 8, f)
}

/// Links to the space record the PRs of its branches (open, merged, or closed) and the PRs at
/// `session_pr_urls` (the PR links its agent sessions made), and updates the title and state
/// of those it has. Only a PR whose head branch the space checks out counts. Returns how many
/// are new. The caller saves the record. With the kind off it changes nothing.
pub fn link_space_prs(rec: &mut Value, session_pr_urls: &[String]) -> Result<usize> {
    let Some(host) = current()? else { return Ok(0) };
    let mut prs = crate::core::prs_of(rec);
    let known: Vec<String> = prs.iter().filter_map(|x| x["key"].as_str().map(String::from)).collect();
    let checkouts = rec["checkouts"].as_array().cloned().unwrap_or_default();

    let mut wanted = Vec::new();
    for k in &checkouts {
        let path = Path::new(k["path"].as_str().unwrap_or_default());
        if !path.exists() {
            continue;
        }
        if let (Some(repo), Some(branch)) = (host.org_repo(&origin_of(path)), k["branch"].as_str()) {
            wanted.push(Want::Branch(repo, branch.to_string()));
        }
    }
    for url in session_pr_urls {
        if let Some((repo, number)) = host.pr_ref(url) {
            if !known.contains(&format!("{repo}#{number}")) {
                wanted.push(Want::Url(url.clone()));
            }
        }
    }
    let found: Vec<Value> = par_map(&wanted, |w| match w {
        Want::Branch(repo, branch) => host.branch_prs(repo, branch).into_iter().map(|p| with_repo(p, repo)).collect(),
        Want::Url(url) => host.pr_at(url).into_iter().collect::<Vec<Value>>(),
    })
    .into_iter()
    .flatten()
    .collect();

    // A session can name PRs of other work (a worktree used for several branches in turn).
    let branches: Vec<&str> = checkouts.iter().filter_map(|k| k["branch"].as_str()).collect();
    let mut found: Vec<Value> = found.into_iter().filter(|p| p["headRefName"].as_str().is_some_and(|b| branches.contains(&b))).collect();
    found.sort_by_key(|p| p["number"].as_u64().unwrap_or(0));
    let new = merge_found(&mut prs, &found);
    rec["prs"] = Value::Array(prs);
    if let Some(o) = rec.as_object_mut() {
        o.remove("pr");
    }
    Ok(new)
}

/// What link_space_prs asks the host: the PRs of a repo's branch, or the PR at a URL.
enum Want {
    Branch(String, String),
    Url(String),
}

fn with_repo(mut p: Value, repo: &str) -> Value {
    p["repo"] = json!(repo.to_lowercase());
    p
}

/// Adds the found PRs that are new, updates the title and state of the known ones, and
/// returns how many are new.
fn merge_found(prs: &mut Vec<Value>, found: &[Value]) -> usize {
    let mut new = 0;
    for p in found {
        let key = format!("{}#{}", p["repo"].as_str().unwrap_or_default(), p["number"]);
        if let Some(old) = prs.iter_mut().find(|x| x["key"].as_str() == Some(key.as_str())) {
            old["title"] = p["title"].clone();
            old["state"] = p["state"].clone();
            continue;
        }
        prs.push(json!({
            "key": key, "repo": p["repo"], "number": p["number"], "title": p["title"], "url": p["url"],
            "author": p["author"]["login"], "kind": "work", "state": p["state"],
        }));
        new += 1;
    }
    new
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_key_matches_ssh_and_https() {
        let want = "github.com/acme/web";
        assert_eq!(remote_key("git@github.com:Acme/web.git"), want);
        assert_eq!(remote_key("https://github.com/acme/web.git\n"), want);
        assert_eq!(remote_key("https://user@github.com/acme/web"), want);
        assert_eq!(remote_key("ssh://git@github.com/acme/web.git"), want);
        assert_eq!(remote_key(""), "");
    }

    #[test]
    fn merge_found_adds_new_and_updates_known() {
        let mut prs = vec![json!({ "key": "web#1", "title": "old", "state": "OPEN" })];
        let found = vec![
            json!({ "repo": "web", "number": 1, "title": "new", "state": "MERGED" }),
            json!({ "repo": "web", "number": 2, "title": "two", "state": "OPEN", "url": "u", "author": { "login": "ann" } }),
        ];
        assert_eq!(merge_found(&mut prs, &found), 1);
        assert_eq!(prs[0]["title"], "new");
        assert_eq!(prs[0]["state"], "MERGED");
        assert_eq!(prs[1]["key"], "web#2");
        assert_eq!(prs[1]["author"], "ann");
        assert_eq!(prs[1]["kind"], "work");
    }

    #[test]
    fn par_map_keeps_order() {
        let items: Vec<u32> = (0..20).collect();
        assert_eq!(par_map(&items, |x| x * 2), (0..20).map(|x| x * 2).collect::<Vec<_>>());
        assert!(par_map(&Vec::<u32>::new(), |x| *x).is_empty());
    }
}
