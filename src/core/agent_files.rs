//! The files that tell an agent about its space: CLAUDE.md, and the skills in .claude/skills.

use super::checkout::copy_tree;
use super::config::{project, project_repos, project_root};
use super::text;
use crate::paths::templates_dir;
use crate::util::{expand, git};
use anyhow::{anyhow, Result};
use regex::{Captures, Regex};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Fills a template: $name and ${name} come from `vars`, $$ becomes $, and every other $ stays
/// as it is. An unknown name is not an error.
fn substitute(template: &str, vars: &HashMap<&str, String>) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\$(?:(\$)|([_a-zA-Z][_a-zA-Z0-9]*)|\{([_a-zA-Z][_a-zA-Z0-9]*)\})").expect("valid regex"));
    re.replace_all(template, |caps: &Captures| {
        if caps.get(1).is_some() {
            return "$".to_string();
        }
        let name = caps.get(2).or_else(|| caps.get(3)).map_or("", |m| m.as_str());
        vars.get(name).cloned().unwrap_or_else(|| caps[0].to_string())
    })
    .into_owned()
}

/// The rows of the checkouts table in CLAUDE.md.
fn checkout_rows(rec: &Value) -> String {
    let rows: Vec<String> = rec["checkouts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|k| {
            let kind = text(k, "label").map_or_else(|| "main".to_string(), |l| format!("extra, for {l}"));
            let s = |key: &str| k[key].as_str().unwrap_or("").to_string();
            format!("| `{}/` | {} | `{}` | {} |", s("folder"), s("repo"), s("branch"), kind)
        })
        .collect();
    if rows.is_empty() {
        "| (none yet) | | | |".to_string()
    } else {
        rows.join("\n")
    }
}

/// Writes CLAUDE.md and the built-in skills of a space, and links the skills both ways.
/// Returns what changed: "CLAUDE.md", "skill: <name>", and what `link_skills` linked.
pub fn write_agent_files(c: &Value, rec: &Value, space_dir: &Path) -> Result<Vec<String>> {
    let p = project(c, rec["project"].as_str().unwrap_or(""))?;
    let have: Vec<&str> = rec["checkouts"].as_array().into_iter().flatten().filter_map(|k| k["repo"].as_str()).collect();
    let more: Vec<String> =
        project_repos(p).into_iter().filter(|(r, _)| !have.contains(&r.as_str())).map(|(r, _)| format!("`{r}`")).collect();
    let template = text(p, "claude_md").map_or_else(|| templates_dir().join("CLAUDE.md"), expand);
    let template = std::fs::read_to_string(&template).map_err(|e| anyhow!("{}: {e}", template.display()))?;
    let vars = HashMap::from([
        ("space", rec["name"].as_str().unwrap_or("").to_string()),
        ("project", rec["project"].as_str().unwrap_or("").to_string()),
        ("goal", text(rec, "goal").unwrap_or("Not given. Ask the user.").to_string()),
        ("space_dir", space_dir.to_string_lossy().into_owned()),
        ("root", project_root(p).to_string_lossy().into_owned()),
        ("checkouts", checkout_rows(rec)),
        ("available", if more.is_empty() { "none".to_string() } else { more.join(", ") }),
    ]);
    let text = substitute(&template, &vars);
    let mut changed = Vec::new();
    let claude_md = space_dir.join("CLAUDE.md");
    if std::fs::read_to_string(&claude_md).ok().as_deref() != Some(text.as_str()) {
        std::fs::write(&claude_md, text)?;
        changed.push("CLAUDE.md".to_string());
    }

    let skills = space_dir.join(".claude").join("skills");
    std::fs::create_dir_all(&skills)?;
    for d in sorted_entries(&templates_dir().join("skills")) {
        if d.is_dir() {
            let name = file_name(&d);
            if copy_tree(&d, &skills.join(&name))? {
                changed.push(format!("skill: {name}"));
            }
        }
    }
    changed.extend(link_skills(rec, space_dir)?);
    Ok(changed)
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir).map(|e| e.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    found.sort();
    found
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Where a link points, also when its target is gone; else the path itself.
fn resolve(p: &Path) -> PathBuf {
    if let Ok(r) = p.canonicalize() {
        return r;
    }
    match std::fs::read_link(p) {
        Ok(t) if t.is_relative() => p.parent().map_or(t.clone(), |d| d.join(&t)),
        Ok(t) => t,
        Err(_) => p.to_path_buf(),
    }
}

/// Claude sees only ~/.claude/skills and the .claude/skills of the folder it starts in, not of a
/// parent or a child folder. So an agent in the space and an agent in a checkout see the same
/// skills: each checkout's own skills get a link in the space, and the space's skills (the
/// built-in ones) a link in each checkout. A name that is there already stays. The links in a
/// checkout go into the repo's info/exclude, so they are never changes.
/// Returns what it linked, as "<to>: <skill>".
pub fn link_skills(rec: &Value, space_dir: &Path) -> Result<Vec<String>> {
    let skills = space_dir.join(".claude").join("skills");
    std::fs::create_dir_all(&skills)?;
    // The space's own skills, without the links to a checkout's skills: one repo's skills
    // do not go on to the other repos.
    let inside = space_dir.canonicalize().unwrap_or_else(|_| space_dir.to_path_buf());
    let own: Vec<PathBuf> = sorted_entries(&skills).into_iter().filter(|d| !(d.is_symlink() && resolve(d).starts_with(&inside))).collect();
    let mut done = Vec::new();
    for k in rec["checkouts"].as_array().into_iter().flatten() {
        let dest = PathBuf::from(k["path"].as_str().unwrap_or(""));
        if !dest.is_dir() {
            continue;
        }
        let theirs = dest.join(".claude").join("skills");
        for d in sorted_entries(&theirs) {
            let link = skills.join(file_name(&d));
            if !d.is_symlink() && d.join("SKILL.md").is_file() && link.symlink_metadata().is_err() {
                std::os::unix::fs::symlink(&d, &link)?;
                done.push(format!("space: {}", file_name(&d)));
            }
        }
        let mut linked = Vec::new();
        for d in &own {
            let t = theirs.join(file_name(d));
            if t.symlink_metadata().is_ok() {
                continue;
            }
            std::fs::create_dir_all(&theirs)?;
            std::os::unix::fs::symlink(resolve(d), &t)?;
            linked.push(file_name(d));
            done.push(format!("{}: {}", k["folder"].as_str().unwrap_or(""), file_name(d)));
        }
        if !linked.is_empty() {
            exclude_links(&dest, &linked)?;
        }
    }
    Ok(done)
}

/// Adds the skill links of a checkout to its repo's info/exclude.
fn exclude_links(checkout: &Path, linked: &[String]) -> Result<()> {
    let common = git(checkout, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let exclude = PathBuf::from(common).join("info").join("exclude");
    let text = std::fs::read_to_string(&exclude).unwrap_or_default();
    let have: Vec<&str> = text.lines().collect();
    let add: Vec<String> = linked.iter().map(|n| format!("/.claude/skills/{n}")).filter(|l| !have.contains(&l.as_str())).collect();
    if add.is_empty() {
        return Ok(());
    }
    if let Some(dir) = exclude.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut lines: Vec<&str> = have;
    lines.push("# herdr-helm: space skills linked into checkouts");
    lines.extend(add.iter().map(String::as_str));
    std::fs::write(&exclude, lines.join("\n") + "\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn substitute_is_safe() {
        let vars = HashMap::from([("space", "s1".to_string()), ("goal", "fix $x".to_string())]);
        assert_eq!(substitute("# $space: ${goal}", &vars), "# s1: fix $x");
        assert_eq!(substitute("$unknown and $$5 and $ alone", &vars), "$unknown and $5 and $ alone");
        assert_eq!(substitute("${space}x $spacex", &vars), "s1x $spacex");
    }

    #[test]
    fn rows_of_checkouts() {
        let rec = json!({ "checkouts": [
            { "folder": "web", "repo": "web", "branch": "s1", "label": null },
            { "folder": "web.api", "repo": "web", "branch": "s1-api", "label": "api" },
        ] });
        assert_eq!(checkout_rows(&rec), "| `web/` | web | `s1` | main |\n| `web.api/` | web | `s1-api` | extra, for api |");
        assert_eq!(checkout_rows(&json!({ "checkouts": [] })), "| (none yet) | | | |");
    }
}
